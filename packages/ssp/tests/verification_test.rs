mod common;

use common::ViewUpdateExt;
use ssp::{Circuit, QueryPlan};
use ssp::engine::view::{Operator, Predicate, Path};
use serde_json::json;

#[test]
fn test_dependency_graph_optimization() {
    let mut circuit = Circuit::new();

    // 1. Create a View dependent on "users" table
    let plan = QueryPlan {
        id: "view_user_100".to_string(),
        root: Operator::Filter {
            input: Box::new(Operator::Scan { table: "users".to_string() }),
            predicate: Predicate::Gt {
                field: Path::new("age"),
                value: json!(100),
            }
        }
    };
    circuit.register_view(plan, None, None);

    // 2. Create another view dependent on "products"
    let plan2 = QueryPlan {
        id: "view_products_cheap".to_string(),
        root: Operator::Filter {
            input: Box::new(Operator::Scan { table: "products".to_string() }),
            predicate: Predicate::Lt {
                field: Path::new("price"),
                value: json!(10),
            }
        }
    };
    circuit.register_view(plan2, None, None);

    // 3. Verify Dependency Graph
    // "users" -> [0], "products" -> [1]
    assert_eq!(circuit.dependency_graph.get("users").unwrap().len(), 1);
    assert_eq!(circuit.dependency_graph.get("products").unwrap().len(), 1);
    
    // 4. Ingest Batch affecting ONLY "users"
    // The "products" view should NOT be processed.
    let batch = vec![
        ("users".to_string(), "CREATE".to_string(), "users:1".to_string(), json!({"id": "users:1", "age": 105}), "hash1".to_string()),
    ];

    let updates = circuit.ingest_batch_outdated(batch);

    // Should have update for "view_user_100"
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].query_id(), "view_user_100");

    println!("2-Phase Batching Verification Passed!");
}
