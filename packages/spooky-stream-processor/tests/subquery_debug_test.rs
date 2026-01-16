use common::ViewUpdateExt;
mod common;

use common::*;
use serde_json::json;
use spooky_stream_processor::engine::view::{Operator, Path, Predicate, Projection, QueryPlan};

/// Debug test for subquery projection children population.
/// Tests: SELECT *, (SELECT * FROM author WHERE id = $parent.author)[0] as author_data FROM thread
#[test]
fn test_subquery_projection_children() {
    let mut circuit = setup();

    // 1. Setup: Create author and thread
    let (author_id, author_record) = make_author_record("Alice");
    ingest(&mut circuit, "author", "CREATE", &author_id, author_record);

    let (thread_id, thread_record) = make_thread_record("Hello World", &author_id);
    ingest(&mut circuit, "thread", "CREATE", &thread_id, thread_record);

    // 2. Build query plan with subquery projection
    // This mimics: SELECT *, (SELECT * FROM author WHERE id = $parent.author)[0] as author_data FROM thread

    // Subquery: SELECT * FROM author WHERE id = $parent.author LIMIT 1
    let subquery_op = Operator::Limit {
        input: Box::new(Operator::Filter {
            input: Box::new(Operator::Scan {
                table: "author".to_string(),
            }),
            predicate: Predicate::Eq {
                field: Path::new("id"),
                value: json!({ "$param": "parent.author" }),
            },
        }),
        limit: 1,
        order_by: None,
    };

    // Main query: SELECT *, subquery FROM thread
    let main_op = Operator::Project {
        input: Box::new(Operator::Scan {
            table: "thread".to_string(),
        }),
        projections: vec![
            Projection::All,
            Projection::Subquery {
                alias: "author_data".to_string(),
                plan: Box::new(subquery_op),
            },
        ],
    };

    let plan = QueryPlan {
        id: "thread_with_author_subquery".to_string(),
        root: main_op,
    };

    // 3. Register view
    let update = circuit.register_view(plan, None, None);
    assert!(update.is_some(), "Expected view update");

    let view_update = update.unwrap();
    println!("=== View Update ===");
    println!("query_id: {}", view_update.query_id());
    println!("result_data: {:?}", view_update.result_data());
    println!("result_hash: {}", match &view_update { spooky_stream_processor::ViewUpdate::Flat(f) | spooky_stream_processor::ViewUpdate::Tree(f) => &f.result_hash, _ => panic!("Expected Flat update") });

    // 4. Verify result contains BOTH the thread AND the author (from subquery)
    assert!(!view_update.result_data().is_empty(), "Expected results");

    // Extract IDs from result_data
    let result_ids: Vec<&str> = view_update
        .result_data()
        .iter()
        .map(|(id, _): (&String, &u64)| id.as_str())
        .collect();

    assert!(
        result_ids.contains(&thread_id.as_str()),
        "Expected thread ID in result"
    );
    assert!(
        result_ids.contains(&author_id.as_str()),
        "Expected author ID from subquery in result"
    );

    // 5. Verify we have both IDs (thread + author = 2 records)
    assert_eq!(
        view_update.result_data().len(),
        2,
        "Expected 2 records (thread + author from subquery)"
    );

    println!("[TEST] ✓ Subquery test passed - includes joined children!");
}

/// Helper to create author record (similar to common but returns Value)
fn make_author_record(name: &str) -> (String, serde_json::Value) {
    let id = generate_id();
    let full_id = format!("author:{}", id);
    let record = json!({
        "id": full_id,
        "name": name,
        "type": "author"
    });
    (full_id, record)
}

/// Helper to create thread record
fn make_thread_record(title: &str, author_id: &str) -> (String, serde_json::Value) {
    let id = generate_id();
    let full_id = format!("thread:{}", id);
    let record = json!({
        "id": full_id,
        "title": title,
        "author": author_id,
        "type": "thread"
    });
    (full_id, record)
}
