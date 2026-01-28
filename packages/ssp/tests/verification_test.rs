mod common;


use ssp::{Circuit, QueryPlan};
use ssp::{Operator, Predicate, Path};
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
    assert_eq!(circuit.dependency_list.get("users").unwrap().len(), 1);
    assert_eq!(circuit.dependency_list.get("products").unwrap().len(), 1);
    
    // 4. Ingest Batch affecting ONLY "users"
    // The "products" view should NOT be processed.
    let batch = vec![
        ssp::engine::circuit::dto::BatchEntry::create("users", "users:1", json!({"id": "users:1", "age": 105}).into()),
    ];

    let updates = circuit.ingest_batch(batch);

    // Should have update for "view_user_100"
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].query_id(), "view_user_100");

    println!("2-Phase Batching Verification Passed!");
}
