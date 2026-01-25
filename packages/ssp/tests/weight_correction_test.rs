use common::setup;
use serde_json::json;
use smol_str::SmolStr;
use ssp::engine::types::{BatchDeltas, Delta, Operation};
use ssp::{Operator, Projection, QueryPlan};
use ssp::engine::update::{DeltaEvent, ViewUpdate, ViewResultFormat};
use ssp::engine::view::View;
use ssp::engine::circuit::Database;

mod common;

/// Test: When two threads reference the same user, user should have weight 2
#[test]
fn test_subquery_weight_accumulation() {
    let mut db = Database::new();

    // Create user:1 (Weight = 1)
    let users = db.ensure_table("user");
    users.rows.insert(SmolStr::new("user:1"), json!({"id": "user:1", "name": "Alice"}).into());
    users.zset.insert(SmolStr::new("user:1"), 1);

    // Create 2 threads referencing user:1 (Weight = 1 each)
    let threads = db.ensure_table("thread");
    threads.rows.insert(SmolStr::new("thread:1"), json!({"id": "thread:1", "author": "user:1"}).into());
    threads.rows.insert(SmolStr::new("thread:2"), json!({"id": "thread:2", "author": "user:1"}).into());
    threads.zset.insert(SmolStr::new("thread:1"), 1);
    threads.zset.insert(SmolStr::new("thread:2"), 1);

    // View: SELECT *, (SELECT * FROM user WHERE id=$parent.author) FROM thread
    let plan = QueryPlan {
        id: "thread_with_author".to_string(),
        root: Operator::Project {
            input: Box::new(Operator::Scan { table: "thread".to_string() }),
            projections: vec![
                Projection::All,
                Projection::Subquery {
                    alias: "author".to_string(),
                    plan: Box::new(Operator::Filter {
                        input: Box::new(Operator::Scan { table: "user".to_string() }),
                        predicate: ssp::Predicate::Eq {
                            field: ssp::Path::new("id"),
                            value: json!({"$param": "parent.author"}),
                        }
                    })
                }
            ]
        }
    };

    let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));

    // Process initial batch
    let update = view.process_batch(&BatchDeltas::new(), &db);

    // Verify weights in cache
    // Thread weights should be 1
    assert_eq!(view.cache.get("thread:1").copied(), Some(1));
    assert_eq!(view.cache.get("thread:2").copied(), Some(1));

    // User weight should be 2 (referenced twice)
    assert_eq!(view.cache.get("user:1").copied(), Some(2), "User:1 should have weight 2 because it is referenced by 2 threads");

    // Verify Streaming Output: Only ONE 'Created' event for user:1
    if let Some(ViewUpdate::Streaming(s)) = update {
        let user_creates: Vec<_> = s.records.iter()
            .filter(|r| r.id == "user:1" && r.event == DeltaEvent::Created)
            .collect();
        assert_eq!(user_creates.len(), 1, "Should emit exactly one Created event for user:1");
    } else {
        panic!("Expected Streaming update");
    }
}

/// Test: When one thread is deleted, user weight decreases but user stays
#[test]
fn test_subquery_weight_decrease_no_removal() {
    let mut db = Database::new();
    
    // Setup initial state: 2 threads -> 1 user
    let users = db.ensure_table("user");
    users.rows.insert(SmolStr::new("user:1"), json!({"id": "user:1"}).into());
    users.zset.insert(SmolStr::new("user:1"), 1);

    let threads = db.ensure_table("thread");
    threads.rows.insert(SmolStr::new("thread:1"), json!({"id": "thread:1", "author": "user:1"}).into());
    threads.rows.insert(SmolStr::new("thread:2"), json!({"id": "thread:2", "author": "user:1"}).into());
    threads.zset.insert(SmolStr::new("thread:1"), 1);
    threads.zset.insert(SmolStr::new("thread:2"), 1);

    let plan = QueryPlan {
        id: "thread_with_author".to_string(),
        root: Operator::Project {
            input: Box::new(Operator::Scan { table: "thread".to_string() }),
            projections: vec![
                Projection::All,
                Projection::Subquery {
                    alias: "author".to_string(),
                    plan: Box::new(Operator::Filter {
                        input: Box::new(Operator::Scan { table: "user".to_string() }),
                        predicate: ssp::Predicate::Eq {
                            field: ssp::Path::new("id"),
                            value: json!({"$param": "parent.author"}),
                        }
                    })
                }
            ]
        }
    };

    let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
    view.process_batch(&BatchDeltas::new(), &db); // Init (weight 2)

    // DELETE thread:1
    let delta = Delta::from_operation(
        SmolStr::new("thread"),
        SmolStr::new("thread:1"),
        Operation::Delete,
    );
    
    // Apply deletion to DB
    {
        let threads = db.ensure_table("thread");
        threads.zset.remove("thread:1");
        threads.rows.remove("thread:1");
    }

    // Process update
    let update = view.process_delta(&delta, &db);

    // Verify weights
    assert_eq!(view.cache.get("user:1").copied(), Some(1), "User weight should decrease to 1");
    
    // No 'Deleted' event because user is still present via thread:2
    if let Some(ViewUpdate::Streaming(s)) = update {
        assert!(!s.records.iter().any(|r| r.id == "user:1" && r.event == DeltaEvent::Deleted), 
            "Should NOT emit Deleted event for user:1 while it still has weight 1");
    }
}

/// Test: When last thread referencing user is deleted, user is removed
#[test]
fn test_subquery_weight_to_zero_removal() {
    let mut db = Database::new();
    
    // Setup initial state: 1 thread -> 1 user
    {
        let users = db.ensure_table("user");
        users.rows.insert(SmolStr::new("user:1"), json!({"id": "user:1"}).into());
        users.zset.insert(SmolStr::new("user:1"), 1);

        let threads = db.ensure_table("thread");
        threads.rows.insert(SmolStr::new("thread:1"), json!({"id": "thread:1", "author": "user:1"}).into());
        threads.zset.insert(SmolStr::new("thread:1"), 1);
    }

    let plan = QueryPlan {
        id: "thread_with_author".to_string(),
        root: Operator::Project {
            input: Box::new(Operator::Scan { table: "thread".to_string() }),
            projections: vec![
                Projection::All,
                Projection::Subquery {
                    alias: "author".to_string(),
                    plan: Box::new(Operator::Filter {
                        input: Box::new(Operator::Scan { table: "user".to_string() }),
                        predicate: ssp::Predicate::Eq {
                            field: ssp::Path::new("id"),
                            value: json!({"$param": "parent.author"}),
                        }
                    })
                }
            ]
        }
    };

    let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
    view.process_batch(&BatchDeltas::new(), &db); // Init (weight 1)

    // DELETE thread:1
    let delta = Delta::from_operation(
        SmolStr::new("thread"),
        SmolStr::new("thread:1"),
        Operation::Delete,
    );

    // Apply deletion to DB
    {
        let threads = db.ensure_table("thread");
        threads.zset.remove("thread:1");
        threads.rows.remove("thread:1");
    }

    let update = view.process_delta(&delta, &db);

    // Verify removed
    assert!(view.cache.get("user:1").is_none(), "User:1 should be removed from cache");

    // Expect Deleted event
    if let Some(ViewUpdate::Streaming(s)) = update {
        assert!(s.records.iter().any(|r| r.id == "user:1" && r.event == DeltaEvent::Deleted), 
            "Should emit Deleted event for user:1");
    } else {
        panic!("Expected Streaming update");
    }
}
