mod common;

use common::{*, ViewUpdateExt};
use serde_json::json;
use ssp::engine::update::ViewUpdate;

/// Test to reproduce the bug where creating a comment did not trigger an update
/// for the thread view because the thread record itself didn't change.
#[test]
fn test_comment_creation_updates_thread_view() {
    let mut circuit = setup();

    // 1. Create Data
    let author_id = create_author(&mut circuit, "Alice");
    let thread_id = create_thread(&mut circuit, "My Thread", &author_id);

    println!("Created Thread: {}", thread_id);

    // 2. Register View with Subquery
    // Query: SELECT *, (SELECT * FROM comment WHERE thread = $parent.id) AS comments FROM thread WHERE id = $id
    let sql = "SELECT *, (SELECT * FROM comment WHERE thread = $parent.id) AS comments FROM thread WHERE id = $id";

    let config = json!({
        "id": "view_thread_detail",
        "surql": sql,
        "params": {
            "id": thread_id
        },
        "clientId": "test-client",
        "ttl": "3600s",
        "lastActiveAt": "2026-01-16T00:00:00Z"
    });

    let data = ssp::service::view::prepare_registration(config)
        .expect("Registration failed");

    // Initial Register
    let update = circuit
        .register_view(data.plan, data.safe_params, None)
        .expect("Initial view update failed");

    // Verify initial state (0 comments)
    // Verify initial state (1 record: the thread)
    // Streaming updates return DeltaRecords, not a simple list of IDs via result_data()
    let initial_ids: Vec<String> = match &update {
        ViewUpdate::Streaming(s) => s.records.iter().map(|r| r.id.to_string()).collect(),
        _ => update.result_data().iter().map(|s| s.to_string()).collect(),
    };
    
    println!("Initial Result IDs: {:?}", initial_ids);
    assert!(!initial_ids.is_empty(), "Initial update should contain records");

    // 3. Create Comment
    println!("\n--- Creating Comment ---");
    let (comment_id, comment_record) = make_comment_record("Use Rust!", &thread_id, &author_id);

    // Ingest the comment
    // This calls circuit.ingest_record -> ingest_batch_spooky -> process_ingest
    let updates = ingest(
        &mut circuit,
        "comment",
        "CREATE",
        &comment_id,
        comment_record,
    );

    println!("Updates received: {:?}", updates);

    // 4. Verify Update
    // WITHOUT THE FIX: `updates` would be empty because the thread row didn't change.
    // WITH THE FIX: `updates` should contain an update for the view.
    assert!(
        !updates.is_empty(),
        "Expected view update after creating comment"
    );

    let view_update = &updates[0];
    assert_eq!(view_update.query_id(), "view_thread_detail");

    // The result_data should ideally contain the thread ID (and its new hash).
    // The exact content of result_data depends on how subquery results are flattened or hashed.
    // But getting ANY update confirms the fix.

    println!("Test Passed: Received update for comment creation");
}
