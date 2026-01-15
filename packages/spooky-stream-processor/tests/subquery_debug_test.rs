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
    let update = circuit.register_view(plan, None);
    assert!(update.is_some(), "Expected view update");

    let view_update = update.unwrap();
    println!("=== View Update ===");
    println!("query_id: {}", view_update.query_id);
    println!("result_ids: {:?}", view_update.result_ids);
    println!("result_hash: {}", view_update.result_hash);
    println!(
        "tree: {}",
        serde_json::to_string_pretty(&view_update.tree).unwrap()
    );

    // 4. Verify result contains the thread
    assert!(
        view_update.result_ids.contains(&thread_id),
        "Expected thread in result_ids"
    );

    // 5. Check that tree has leaves with children
    if let Some(leaves) = &view_update.tree.leaves {
        assert!(!leaves.is_empty(), "Expected leaves");
        let leaf = &leaves[0];
        println!("Leaf: {:?}", leaf);

        // THE KEY CHECK: children should contain author_data
        assert!(leaf.children.is_some(), "Expected children to be populated");
        if let Some(children) = &leaf.children {
            assert!(
                children.contains_key("author_data"),
                "Expected 'author_data' in children"
            );
            let author_tree = &children["author_data"];
            println!(
                "author_data tree: {}",
                serde_json::to_string_pretty(author_tree).unwrap()
            );
        }
    } else {
        panic!("Expected leaves in tree");
    }
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
