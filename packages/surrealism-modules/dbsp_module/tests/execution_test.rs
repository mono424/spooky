use dbsp_module::{Circuit, Operator, QueryPlan, JoinCondition, MaterializedViewUpdate};
use serde_json::{json, Value};
use std::collections::HashMap;

#[test]
fn test_join_execution() {
    let mut circuit = Circuit::new();
    
    // Plan: Join users and posts on users.id = posts.author
    let plan = QueryPlan {
        id: "join_view".to_string(),
        root: Operator::Join {
            left: Box::new(Operator::Scan { table: "user".to_string() }),
            right: Box::new(Operator::Scan { table: "post".to_string() }),
            on: JoinCondition {
                left_field: "id".to_string(),
                right_field: "author".to_string(),
            },
        },
    };
    circuit.register_view(plan);

    // 1. Data Ingest: Users
    // user:1 (id=1)
    let delta_u: HashMap<String, i64> = vec![
        ("user:1".to_string(), 1),
        ("user:2".to_string(), 1)
    ].into_iter().collect();
    
    let _ = circuit.step("user".to_string(), delta_u);
    circuit.db.ensure_table("user").update_row("user:1".to_string(), json!({"id": 1, "name": "Alice"}));
    circuit.db.ensure_table("user").update_row("user:2".to_string(), json!({"id": 2, "name": "Bob"}));

    // 2. Data Ingest: Posts
    // post:10 (author=1)
    let delta_p: HashMap<String, i64> = vec![
        ("post:10".to_string(), 1),
        ("post:11".to_string(), 1), 
        ("post:12".to_string(), 1) // author=3 (no match)
    ].into_iter().collect();

    circuit.db.ensure_table("post").update_row("post:10".to_string(), json!({"id": 10, "author": 1, "title": "Hello"}));
    circuit.db.ensure_table("post").update_row("post:11".to_string(), json!({"id": 11, "author": 1, "title": "World"}));
    circuit.db.ensure_table("post").update_row("post:12".to_string(), json!({"id": 12, "author": 3, "title": "Draft"}));

    // Run step for posts
    let updates = circuit.step("post".to_string(), delta_p);
    
    // Expect: user:1 matches (post:10, post:11).
    // Result keys: user:1 (weight 2).
    // user:2 (no match).
    
    // Check update
    assert_eq!(updates.len(), 1);
    let view_up = &updates[0];
    assert_eq!(view_up.query_id, "join_view");
    
    // Result IDs should contain "user:1".
    // Wait, Join output keys.
    // My Join implementation in `eval_snapshot`:
    // *out.entry(l_key.clone()) += w;
    // It returns LEFT keys (user:1).
    // So if user:1 matches 2 posts, weight is 2.
    // ZSet keys are unique. So "user:1" is present.
    // Result IDs list: ["user:1"].
    assert!(view_up.result_ids.contains(&"user:1".to_string()));
    assert!(!view_up.result_ids.contains(&"user:2".to_string())); // No posts for author 2 yet
    assert_eq!(view_up.result_ids.len(), 1);

    // 3. Incremental Update: Add post for user 2
    let delta_p2: HashMap<String, i64> = vec![("post:13".to_string(), 1)].into_iter().collect();
    circuit.db.ensure_table("post").update_row("post:13".to_string(), json!({"id": 13, "author": 2}));

    let updates2 = circuit.step("post".to_string(), delta_p2);
    
    // Now user:2 should appear.
    assert_eq!(updates2.len(), 1);
    let ids = &updates2[0].result_ids;
    assert!(ids.contains(&"user:1".to_string()));
    assert!(ids.contains(&"user:2".to_string()));
    assert_eq!(ids.len(), 2);
}

#[test]
fn test_limit_execution() {
    let mut circuit = Circuit::new();
    
    let plan = QueryPlan {
        id: "limit_view".to_string(),
        root: Operator::Limit {
            limit: 3,
            input: Box::new(Operator::Scan { table: "items".to_string() }),
        },
    };
    circuit.register_view(plan);

    // Insert 5 items
    let mut delta: HashMap<String, i64> = HashMap::new();
    for i in 1..=5 {
        let key = format!("item:{}", i);
        delta.insert(key.clone(), 1);
        circuit.db.ensure_table("items").update_row(key, json!({"val": i}));
    }

    let updates = circuit.step("items".to_string(), delta);
    
    assert_eq!(updates.len(), 1);
    let ids = &updates[0].result_ids;
    
    // Should have 3 items.
    // Since input is sorted by ID (string), expect item:1, item:2, item:3.
    // "item:1", "item:2", "item:3", "item:4", "item:5".
    // 1, 2, 3 comes first?
    assert_eq!(ids.len(), 3);
    assert!(ids.contains(&"item:1".to_string()));
    assert!(ids.contains(&"item:2".to_string()));
    assert!(ids.contains(&"item:3".to_string()));
    assert!(!ids.contains(&"item:4".to_string()));
}
