use common::ViewUpdateExt;
mod common;

use common::*;
use serde_json::json;
use ssp::engine::update::{DeltaEvent, ViewResultFormat, ViewUpdate};

#[test]
fn test_complex_subquery_bug_streaming() {
    let mut circuit = setup();
    
    // 1. Setup Data
    let (alice_id, alice_record) = make_user_record("Alice");
    ingest(&mut circuit, "user", "CREATE", &alice_id, alice_record);
    
    let (bob_id, bob_record) = make_user_record("Bob");
    ingest(&mut circuit, "user", "CREATE", &bob_id, bob_record);

    let (thread_id, thread_record) = make_thread_record("My Thread", &alice_id);
    ingest(&mut circuit, "thread", "CREATE", &thread_id, thread_record);
    
    // Initial comments (5 comments)
    let mut initial_comment_ids = Vec::new();
    for i in 0..5 {
        let timestamp = format!("2024-01-01T10:{:02}:00Z", i);
        let text = format!("Initial Comment {}", i);
        // Bob is author
        let (c_id, c_rec) = make_comment_record_with_time(&text, &thread_id, &bob_id, &timestamp);
        ingest(&mut circuit, "comment", "CREATE", &c_id, c_rec);
        initial_comment_ids.push(c_id);
    }

    // 2. Register View in Streaming Mode
    let sql = "SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author, (SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author FROM comment WHERE thread=$parent.id ORDER BY created_at desc LIMIT 10) AS comments FROM thread WHERE id = $id LIMIT 1";

    let config = json!({
        "id": "streaming_subquery_view",
        "surrealQL": sql,
        "params": {
            "id": thread_id
        },
        "clientId": "test-client",
        "ttl": "3600s",
        "lastActiveAt": "2026-01-24T00:00:00Z"
    });

    let result = ssp::service::view::prepare_registration(config);
    assert!(result.is_ok());
    let data = result.unwrap();

    let initial_update = circuit.register_view(data.plan, data.safe_params, Some(ViewResultFormat::Streaming));
    assert!(initial_update.is_some());
    
    // 3. Verify Initial State
    // Expect: Thread ID, Alice ID (subquery), 5 Comment IDs, Bob ID (subquery for comments)
    let update = initial_update.unwrap();
    if let ViewUpdate::Streaming(s) = &update {
        let ids: Vec<&str> = s.records.iter().map(|r| r.id.as_str()).collect();
        println!("Initial Streaming IDs: {:?}", ids);
        
        assert!(ids.contains(&thread_id.as_str()), "Initial: Should contain thread ID");
        assert!(ids.contains(&alice_id.as_str()), "Initial: Should contain Alice ID (Thread Author)");
        assert!(ids.contains(&bob_id.as_str()), "Initial: Should contain Bob ID (Comment Author)");
        
        for c_id in &initial_comment_ids {
             assert!(ids.contains(&c_id.as_str()), "Initial: Should contain comment {}", c_id);
        }
    } else {
        panic!("Expected Streaming update");
    }

    // 4. Ingest NEW comment with NEW Author
    let (charlie_id, charlie_record) = make_user_record("Charlie");
    ingest(&mut circuit, "user", "CREATE", &charlie_id, charlie_record);

    let timestamp_new = "2024-01-01T10:10:00Z";
    let (new_c_id, new_c_rec) = make_comment_record_with_time("New Comment 6", &thread_id, &charlie_id, timestamp_new);
    let updates = ingest(&mut circuit, "comment", "CREATE", &new_c_id, new_c_rec);
    
    // 5. Verify Update
    let view_updates: Vec<&ViewUpdate> = updates.iter().filter(|u| u.query_id() == "streaming_subquery_view").collect();
    assert!(!view_updates.is_empty(), "Should receive update for view");
    
    let update = view_updates[0];
    if let ViewUpdate::Streaming(s) = update {
        println!("Update received: {:?}", s);
        
        // Assert the new comment ID is present
        let new_comment_record = s.records.iter().find(|r| r.id == new_c_id);
        assert!(new_comment_record.is_some(), "New comment ID should be in the streaming update");
        assert_eq!(new_comment_record.unwrap().event, DeltaEvent::Created);

        // Assert the new author ID (Charlie) is present!
        // The subquery `(SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0]` should bring Charlie into the view.
        let charlie_record_update = s.records.iter().find(|r| r.id == charlie_id);
        assert!(charlie_record_update.is_some(), "New Author ID (Charlie) should be in the streaming update");
        assert_eq!(charlie_record_update.unwrap().event, DeltaEvent::Created, "Charlie should be Created");
    } else {
         panic!("Expected Streaming update");
    }
}


// Helpers

fn make_user_record(name: &str) -> (String, serde_json::Value) {
    let id = generate_id();
    let full_id = format!("user:{}", id);
    let record = json!({
        "id": full_id,
        "name": name,
        "type": "user"
    });
    (full_id, record)
}

fn make_thread_record(title: &str, author_id: &str) -> (String, serde_json::Value) {
    let id = generate_id();
    let full_id = format!("thread:{}", id);
    let record = json!({
        "id": full_id,
        "title": title,
        "author": author_id,
        "created_at": "2024-01-01T10:00:00Z",
        "type": "thread"
    });
    (full_id, record)
}

fn make_comment_record_with_time(text: &str, thread_id: &str, author_id: &str, time: &str) -> (String, serde_json::Value) {
    let id = generate_id();
    let full_id = format!("comment:{}", id);
    let record = json!({
        "id": full_id,
        "text": text,
        "thread": thread_id,
        "author": author_id,
        "created_at": time,
        "type": "comment"
    });
    (full_id, record)
}
