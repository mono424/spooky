use dbsp_module::{Circuit, QueryPlan, Operator, Predicate};
use std::collections::HashMap;

#[test]
fn test_id_tree_generation() {
    let mut circuit = Circuit::new();
    let plan = QueryPlan {
        id: "q_all".to_string(),
        root: Operator::Scan { table: "users".to_string() }
    };
    circuit.register_view(plan);

    // 1. Ingest IDs
    let mut delta = HashMap::new();
    delta.insert("users:1".to_string(), 1);
    delta.insert("users:2".to_string(), 1);
    delta.insert("users:3".to_string(), 1);

    let updates = circuit.step("users".to_string(), delta);
    
    assert_eq!(updates.len(), 1);
    let update = &updates[0];
    assert_eq!(update.query_id, "q_all");
    assert_eq!(update.result_ids, vec!["users:1", "users:2", "users:3"]);
    
    // 2. Verify Tree
    // With 3 items, threshold allows leaf node.
    assert!(update.tree.ids.is_some());
    assert_eq!(update.tree.ids.as_ref().unwrap().len(), 3);
}

#[test]
fn test_filter_and_delete() {
    let mut circuit = Circuit::new();
    let plan = QueryPlan {
        id: "q_admin".to_string(),
        root: Operator::Filter {
            input: Box::new(Operator::Scan { table: "users".to_string() }),
            predicate: Predicate::Prefix { prefix: "users:admin".to_string() }
        }
    };
    circuit.register_view(plan);

    // 1. Ingest Mixed IDs
    let mut delta = HashMap::new();
    delta.insert("users:admin:1".to_string(), 1);
    delta.insert("users:guest:1".to_string(), 1);

    let updates = circuit.step("users".to_string(), delta);
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].result_ids, vec!["users:admin:1"]); // Guest should be filtered out

    // 2. Delete Admin
    let mut delta_delete = HashMap::new();
    delta_delete.insert("users:admin:1".to_string(), -1);
    
    let updates_delete = circuit.step("users".to_string(), delta_delete);
    assert_eq!(updates_delete.len(), 1);
    assert_eq!(updates_delete[0].result_ids.len(), 0); // Should be empty
}
