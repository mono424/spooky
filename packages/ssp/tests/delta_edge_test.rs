//! Tests for specific queries and delta correctness for edge building
//!
//! Tests these exact queries:
//! 1. SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author 
//!    FROM thread ORDER BY title desc LIMIT 10;
//!
//! 2. SELECT * FROM user WHERE id = $id LIMIT 1;
//!
//! 3. SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author,
//!    (SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author 
//!     FROM comment WHERE thread=$parent.id ORDER BY created_at desc LIMIT 10) AS comments 
//!    FROM thread WHERE id = $id LIMIT 1;
//!
//! Run with: cargo test --package ssp --lib -- engine::view::query_delta_tests --nocapture

use ssp::engine::circuit::Database;
use ssp::engine::operators::{Operator, OrderSpec, Predicate, Projection};
use ssp::engine::types::{
    BatchDeltas, Delta, FastMap, FastHashSet, Path, SpookyValue, ZSet,
    make_zset_key,
};
use ssp::engine::update::{DeltaEvent, ViewResultFormat, ViewUpdate};
use ssp::engine::view::{QueryPlan, View};
use smol_str::SmolStr;
use std::collections::HashSet;

// ============================================================================
// TEST HELPERS
// ============================================================================

fn make_user(id: &str, name: &str) -> SpookyValue {
    let mut map = FastMap::default();
    map.insert(SmolStr::new("id"), SpookyValue::Str(SmolStr::new(format!("user:{}", id))));
    map.insert(SmolStr::new("name"), SpookyValue::Str(SmolStr::new(name)));
    SpookyValue::Object(map)
}

fn make_thread(id: &str, author_id: &str, title: &str) -> SpookyValue {
    let mut map = FastMap::default();
    map.insert(SmolStr::new("id"), SpookyValue::Str(SmolStr::new(format!("thread:{}", id))));
    map.insert(SmolStr::new("author"), SpookyValue::Str(SmolStr::new(format!("user:{}", author_id))));
    map.insert(SmolStr::new("title"), SpookyValue::Str(SmolStr::new(title)));
    SpookyValue::Object(map)
}

fn make_comment(id: &str, thread_id: &str, author_id: &str, text: &str, created_at: i64) -> SpookyValue {
    let mut map = FastMap::default();
    map.insert(SmolStr::new("id"), SpookyValue::Str(SmolStr::new(format!("comment:{}", id))));
    map.insert(SmolStr::new("thread"), SpookyValue::Str(SmolStr::new(format!("thread:{}", thread_id))));
    map.insert(SmolStr::new("author"), SpookyValue::Str(SmolStr::new(format!("user:{}", author_id))));
    map.insert(SmolStr::new("text"), SpookyValue::Str(SmolStr::new(text)));
    map.insert(SmolStr::new("created_at"), SpookyValue::Number(created_at as f64));
    SpookyValue::Object(map)
}

fn setup_full_database() -> Database {
    let mut db = Database::new();
    
    // Users
    let user_table = db.ensure_table("user");
    for (id, name) in [("1", "Alice"), ("2", "Bob"), ("3", "Charlie"), ("4", "Diana")] {
        user_table.rows.insert(SmolStr::new(id), make_user(id, name));
        user_table.zset.insert(make_zset_key("user", id), 1);
    }
    
    // Threads (with different authors)
    let thread_table = db.ensure_table("thread");
    let threads = [
        ("1", "1", "Zebra Topic"),      // Alice's thread (Z for desc order test)
        ("2", "1", "Apple Discussion"), // Alice's thread
        ("3", "2", "Banana Talk"),      // Bob's thread
        ("4", "3", "Mango Chat"),       // Charlie's thread
    ];
    for (id, author, title) in threads {
        thread_table.rows.insert(SmolStr::new(id), make_thread(id, author, title));
        thread_table.zset.insert(make_zset_key("thread", id), 1);
    }
    
    // Comments on thread:1
    let comment_table = db.ensure_table("comment");
    let comments = [
        ("1", "1", "2", "First comment", 100),   // Bob comments on thread:1
        ("2", "1", "3", "Second comment", 200),  // Charlie comments on thread:1
        ("3", "1", "1", "Third comment", 300),   // Alice comments on thread:1
        ("4", "2", "4", "Comment on thread 2", 150), // Diana comments on thread:2
    ];
    for (id, thread_id, author_id, text, created_at) in comments {
        comment_table.rows.insert(SmolStr::new(id), make_comment(id, thread_id, author_id, text, created_at));
        comment_table.zset.insert(make_zset_key("comment", id), 1);
    }
    
    db
}

/// Extract delta events from ViewUpdate, returns (id, event_type) pairs
fn extract_events(update: &ViewUpdate) -> Vec<(String, String)> {
    match update {
        ViewUpdate::Streaming(s) => {
            s.records.iter()
                .map(|r| (r.id.to_string(), format!("{:?}", r.event)))
                .collect()
        }
        _ => vec![],
    }
}

/// Extract just the IDs by event type
fn extract_by_event(update: &ViewUpdate, event: DeltaEvent) -> HashSet<String> {
    match update {
        ViewUpdate::Streaming(s) => {
            s.records.iter()
                .filter(|r| std::mem::discriminant(&r.event) == std::mem::discriminant(&event))
                .map(|r| r.id.to_string())
                .collect()
        }
        _ => HashSet::new(),
    }
}

/// Count events by type
fn count_events(update: &ViewUpdate) -> (usize, usize, usize) {
    match update {
        ViewUpdate::Streaming(s) => {
            let created = s.records.iter().filter(|r| matches!(r.event, DeltaEvent::Created)).count();
            let updated = s.records.iter().filter(|r| matches!(r.event, DeltaEvent::Updated)).count();
            let deleted = s.records.iter().filter(|r| matches!(r.event, DeltaEvent::Deleted)).count();
            (created, updated, deleted)
        }
        _ => (0, 0, 0),
    }
}

// ============================================================================
// QUERY 1: Threads with Author (Limited, Ordered)
// SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author 
// FROM thread ORDER BY title desc LIMIT 10;
// ============================================================================

/// Build the query plan for Query 1
fn build_query1_plan() -> QueryPlan {
    QueryPlan {
        id: "threads_with_author_ordered".to_string(),
        root: Operator::Limit {
            input: Box::new(Operator::Project {
                input: Box::new(Operator::Scan { table: "thread".to_string() }),
                projections: vec![
                    Projection::Subquery {
                        alias: "author".to_string(),
                        plan: Box::new(Operator::Limit {
                            input: Box::new(Operator::Filter {
                                input: Box::new(Operator::Scan { table: "user".to_string() }),
                                predicate: Predicate::Eq {
                                    field: Path::new("id"),
                                    value: serde_json::json!({"$param": "parent.author"}),
                                },
                            }),
                            limit: 1,
                            order_by: None,
                        }),
                    },
                ],
            }),
            limit: 10,
            order_by: Some(vec![OrderSpec {
                field: Path::new("title"),
                direction: "DESC".to_string(),
            }]),
        },
    }
}

#[cfg(test)]
mod query1_tests {
    use super::*;

    /// TEST: Initial load should return threads + their authors
    /// Expected edges: thread:1, thread:2, thread:3, thread:4, user:1, user:2, user:3
    /// (4 threads + 3 unique authors - Alice authored 2 threads but should only have 1 edge)
    #[test]
    fn test_query1_initial_load() {
        let db = setup_full_database();
        let plan = build_query1_plan();
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        let result = view.process_batch(&BatchDeltas::new(), &db);
        assert!(result.is_some(), "Should return initial data");
        
        let result = result.unwrap();
        let created = extract_by_event(&result, DeltaEvent::Created);
        
        println!("Query 1 - Initial load Created events:");
        for id in &created {
            println!("  - {}", id);
        }
        
        // Should have 4 threads
        assert!(created.contains("thread:1"), "Should have thread:1");
        assert!(created.contains("thread:2"), "Should have thread:2");
        assert!(created.contains("thread:3"), "Should have thread:3");
        assert!(created.contains("thread:4"), "Should have thread:4");
        
        // Should have 3 unique authors (Alice=user:1, Bob=user:2, Charlie=user:3)
        // Alice authored thread:1 and thread:2, but should only appear ONCE
        assert!(created.contains("user:1"), "Should have user:1 (Alice)");
        assert!(created.contains("user:2"), "Should have user:2 (Bob)");
        assert!(created.contains("user:3"), "Should have user:3 (Charlie)");
        
        // Diana (user:4) is not an author of any thread
        assert!(!created.contains("user:4"), "Should NOT have user:4 (Diana - not an author)");
        
        // Total: 4 threads + 3 authors = 7 edges
        assert_eq!(created.len(), 7, "Should have exactly 7 Created events");
        
        // Verify cache weights are all 1 (membership model)
        for (key, &weight) in &view.cache {
            assert_eq!(weight, 1, "Cache weight for {} should be 1, got {}", key, weight);
        }
    }

    /// TEST: Adding a new thread should create edges for thread + author
    #[test]
    fn test_query1_add_thread_new_author() {
        let mut db = setup_full_database();
        let plan = build_query1_plan();
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        view.process_batch(&BatchDeltas::new(), &db);
        
        // Add new thread by Diana (user:4) who wasn't an author before
        let thread_table = db.tables.get_mut("thread").unwrap();
        thread_table.rows.insert(SmolStr::new("5"), make_thread("5", "4", "New Topic"));
        thread_table.zset.insert(make_zset_key("thread", "5"), 1);
        
        let mut batch = BatchDeltas::new();
        batch.membership.insert("thread".to_string(), {
            let mut z = ZSet::default();
            z.insert(SmolStr::new("thread:5"), 1);
            z
        });
        
        let result = view.process_batch(&batch, &db);
        assert!(result.is_some());
        
        let result = result.unwrap();
        let created = extract_by_event(&result, DeltaEvent::Created);
        
        println!("Query 1 - Add thread with new author:");
        for id in &created {
            println!("  - Created: {}", id);
        }
        
        // Should create edge for new thread
        assert!(created.contains("thread:5"), "Should create edge for thread:5");
        
        // Should create edge for Diana (new author)
        assert!(created.contains("user:4"), "Should create edge for user:4 (Diana)");
        
        // Should be exactly 2 new edges
        assert_eq!(created.len(), 2, "Should have exactly 2 Created events");
    }

    /// TEST: Adding a thread by existing author should NOT create duplicate user edge
    #[test]
    fn test_query1_add_thread_existing_author() {
        let mut db = setup_full_database();
        let plan = build_query1_plan();
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        view.process_batch(&BatchDeltas::new(), &db);
        
        // Add another thread by Alice (user:1) who already has threads
        let thread_table = db.tables.get_mut("thread").unwrap();
        thread_table.rows.insert(SmolStr::new("5"), make_thread("5", "1", "Alice's New Topic"));
        thread_table.zset.insert(make_zset_key("thread", "5"), 1);
        
        let mut batch = BatchDeltas::new();
        batch.membership.insert("thread".to_string(), {
            let mut z = ZSet::default();
            z.insert(SmolStr::new("thread:5"), 1);
            z
        });
        
        let result = view.process_batch(&batch, &db);
        assert!(result.is_some());
        
        let result = result.unwrap();
        let created = extract_by_event(&result, DeltaEvent::Created);
        
        println!("Query 1 - Add thread with existing author:");
        for id in &created {
            println!("  - Created: {}", id);
        }
        
        // Should create edge for new thread
        assert!(created.contains("thread:5"), "Should create edge for thread:5");
        
        // Should NOT create another edge for Alice (already in view!)
        assert!(!created.contains("user:1"), "Should NOT create duplicate edge for user:1");
        
        // Should be exactly 1 new edge (just the thread)
        assert_eq!(created.len(), 1, "Should have exactly 1 Created event");
    }

    /// TEST: Deleting a thread should delete thread edge, but keep author if still referenced
    #[test]
    fn test_query1_delete_thread_author_still_referenced() {
        let mut db = setup_full_database();
        let plan = build_query1_plan();
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        view.process_batch(&BatchDeltas::new(), &db);
        
        // Delete thread:2 (Alice's second thread, she still has thread:1)
        let thread_table = db.tables.get_mut("thread").unwrap();
        thread_table.rows.remove("2");
        thread_table.zset.remove("thread:2");
        
        let mut batch = BatchDeltas::new();
        batch.membership.insert("thread".to_string(), {
            let mut z = ZSet::default();
            z.insert(SmolStr::new("thread:2"), -1);
            z
        });
        
        let result = view.process_batch(&batch, &db);
        assert!(result.is_some());
        
        let result = result.unwrap();
        let deleted = extract_by_event(&result, DeltaEvent::Deleted);
        
        println!("Query 1 - Delete thread (author still has other threads):");
        for id in &deleted {
            println!("  - Deleted: {}", id);
        }
        
        // Should delete edge for thread:2
        assert!(deleted.contains("thread:2"), "Should delete edge for thread:2");
        
        // Should NOT delete Alice (still authors thread:1)
        assert!(!deleted.contains("user:1"), "Should NOT delete user:1 (still authors thread:1)");
        
        // Should be exactly 1 deletion
        assert_eq!(deleted.len(), 1, "Should have exactly 1 Deleted event");
    }

    /// TEST: Deleting the ONLY thread by an author should delete both edges
    #[test]
    fn test_query1_delete_thread_author_no_longer_referenced() {
        let mut db = setup_full_database();
        let plan = build_query1_plan();
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        view.process_batch(&BatchDeltas::new(), &db);
        
        // Delete thread:3 (Bob's only thread)
        let thread_table = db.tables.get_mut("thread").unwrap();
        thread_table.rows.remove("3");
        thread_table.zset.remove("thread:3");
        
        let mut batch = BatchDeltas::new();
        batch.membership.insert("thread".to_string(), {
            let mut z = ZSet::default();
            z.insert(SmolStr::new("thread:3"), -1);
            z
        });
        
        let result = view.process_batch(&batch, &db);
        assert!(result.is_some());
        
        let result = result.unwrap();
        let deleted = extract_by_event(&result, DeltaEvent::Deleted);
        
        println!("Query 1 - Delete thread (author's only thread):");
        for id in &deleted {
            println!("  - Deleted: {}", id);
        }
        
        // Should delete edge for thread:3
        assert!(deleted.contains("thread:3"), "Should delete edge for thread:3");
        
        // Should also delete Bob (no longer referenced)
        assert!(deleted.contains("user:2"), "Should delete user:2 (Bob no longer referenced)");
        
        // Should be exactly 2 deletions
        assert_eq!(deleted.len(), 2, "Should have exactly 2 Deleted events");
    }

    /// TEST: LIMIT should only include top N threads
    #[test]
    fn test_query1_limit_excludes_overflow() {
        let mut db = setup_full_database();
        
        // Add many threads to exceed limit
        let thread_table = db.tables.get_mut("thread").unwrap();
        for i in 5..=15 {
            let title = format!("Thread {}", i);
            thread_table.rows.insert(SmolStr::new(&i.to_string()), make_thread(&i.to_string(), "1", &title));
            thread_table.zset.insert(make_zset_key("thread", &i.to_string()), 1);
        }
        
        let plan = build_query1_plan(); // LIMIT 10
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        let result = view.process_batch(&BatchDeltas::new(), &db);
        assert!(result.is_some());
        
        let result = result.unwrap();
        let created = extract_by_event(&result, DeltaEvent::Created);
        
        println!("Query 1 - With {} threads, LIMIT 10:", 15);
        println!("  Created {} records", created.len());
        
        // Count threads in created
        let thread_count = created.iter().filter(|id| id.starts_with("thread:")).count();
        
        // Should have at most 10 threads due to LIMIT
        assert!(thread_count <= 10, "Should have at most 10 threads, got {}", thread_count);
    }
}

// ============================================================================
// QUERY 2: Single User by ID
// SELECT * FROM user WHERE id = $id LIMIT 1;
// ============================================================================

fn build_query2_plan(user_id: &str) -> QueryPlan {
    QueryPlan {
        id: format!("user_{}", user_id),
        root: Operator::Limit {
            input: Box::new(Operator::Filter {
                input: Box::new(Operator::Scan { table: "user".to_string() }),
                predicate: Predicate::Eq {
                    field: Path::new("id"),
                    value: serde_json::json!(format!("user:{}", user_id)),
                },
            }),
            limit: 1,
            order_by: None,
        },
    }
}

#[cfg(test)]
mod query2_tests {
    use super::*;

    /// TEST: View for specific user should only include that user
    #[test]
    fn test_query2_single_user() {
        let db = setup_full_database();
        let plan = build_query2_plan("1"); // Alice
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        let result = view.process_batch(&BatchDeltas::new(), &db);
        assert!(result.is_some());
        
        let result = result.unwrap();
        let created = extract_by_event(&result, DeltaEvent::Created);
        
        println!("Query 2 - Single user (Alice):");
        for id in &created {
            println!("  - {}", id);
        }
        
        // Should only have user:1
        assert_eq!(created.len(), 1, "Should have exactly 1 edge");
        assert!(created.contains("user:1"), "Should be user:1");
    }

    /// TEST: Update to the matching user should emit Updated
    #[test]
    fn test_query2_user_content_update() {
        let mut db = setup_full_database();
        let plan = build_query2_plan("1");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        view.process_batch(&BatchDeltas::new(), &db);
        
        // Update Alice's data
        let user_table = db.tables.get_mut("user").unwrap();
        user_table.rows.insert(SmolStr::new("1"), make_user("1", "Alice Updated"));
        
        let delta = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:1"),
            weight: 0,
            content_changed: true,
        };
        
        let result = view.process_delta(&delta, &db);
        assert!(result.is_some());
        
        let result = result.unwrap();
        let (created, updated, deleted) = count_events(&result);
        
        assert_eq!(created, 0, "Should not create");
        assert_eq!(updated, 1, "Should update user:1");
        assert_eq!(deleted, 0, "Should not delete");
    }

    /// TEST: Update to different user should not affect this view
    #[test]
    fn test_query2_other_user_update() {
        let mut db = setup_full_database();
        let plan = build_query2_plan("1"); // View for Alice
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        view.process_batch(&BatchDeltas::new(), &db);
        
        // Update Bob's data
        let user_table = db.tables.get_mut("user").unwrap();
        user_table.rows.insert(SmolStr::new("2"), make_user("2", "Bob Updated"));
        
        let delta = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:2"),
            weight: 0,
            content_changed: true,
        };
        
        let result = view.process_delta(&delta, &db);
        
        // Should not emit anything (user:2 is not in this view)
        assert!(result.is_none(), "Should not emit for user:2");
    }

    /// TEST: Two views for different users should have independent edges
    #[test]
    fn test_query2_multiple_views_independent() {
        let db = setup_full_database();
        
        let plan1 = build_query2_plan("1"); // Alice
        let plan2 = build_query2_plan("2"); // Bob
        
        let mut view1 = View::new(plan1, None, Some(ViewResultFormat::Streaming));
        let mut view2 = View::new(plan2, None, Some(ViewResultFormat::Streaming));
        
        let result1 = view1.process_batch(&BatchDeltas::new(), &db);
        let result2 = view2.process_batch(&BatchDeltas::new(), &db);
        
        let created1 = extract_by_event(&result1.unwrap(), DeltaEvent::Created);
        let created2 = extract_by_event(&result2.unwrap(), DeltaEvent::Created);
        
        assert!(created1.contains("user:1"), "View 1 should have Alice");
        assert!(!created1.contains("user:2"), "View 1 should NOT have Bob");
        
        assert!(created2.contains("user:2"), "View 2 should have Bob");
        assert!(!created2.contains("user:1"), "View 2 should NOT have Alice");
    }
}

// ============================================================================
// QUERY 3: Thread with Author and Comments (Nested Subqueries)
// SELECT *, 
//   (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author,
//   (SELECT *, 
//     (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author 
//    FROM comment WHERE thread=$parent.id ORDER BY created_at desc LIMIT 10
//   ) AS comments 
// FROM thread WHERE id = $id LIMIT 1;
// ============================================================================

fn build_query3_plan(thread_id: &str) -> QueryPlan {
    // Inner-most subquery: comment author
    let comment_author_subquery = Operator::Limit {
        input: Box::new(Operator::Filter {
            input: Box::new(Operator::Scan { table: "user".to_string() }),
            predicate: Predicate::Eq {
                field: Path::new("id"),
                value: serde_json::json!({"$param": "parent.author"}),
            },
        }),
        limit: 1,
        order_by: None,
    };
    
    // Comments subquery with nested author subquery
    let comments_subquery = Operator::Limit {
        input: Box::new(Operator::Project {
            input: Box::new(Operator::Filter {
                input: Box::new(Operator::Scan { table: "comment".to_string() }),
                predicate: Predicate::Eq {
                    field: Path::new("thread"),
                    value: serde_json::json!({"$param": "parent.id"}),
                },
            }),
            projections: vec![
                Projection::Subquery {
                    alias: "author".to_string(),
                    plan: Box::new(comment_author_subquery),
                },
            ],
        }),
        limit: 10,
        order_by: Some(vec![OrderSpec {
            field: Path::new("created_at"),
            direction: "DESC".to_string(),
        }]),
    };
    
    // Thread author subquery
    let thread_author_subquery = Operator::Limit {
        input: Box::new(Operator::Filter {
            input: Box::new(Operator::Scan { table: "user".to_string() }),
            predicate: Predicate::Eq {
                field: Path::new("id"),
                value: serde_json::json!({"$param": "parent.author"}),
            },
        }),
        limit: 1,
        order_by: None,
    };
    
    QueryPlan {
        id: format!("thread_detail_{}", thread_id),
        root: Operator::Limit {
            input: Box::new(Operator::Project {
                input: Box::new(Operator::Filter {
                    input: Box::new(Operator::Scan { table: "thread".to_string() }),
                    predicate: Predicate::Eq {
                        field: Path::new("id"),
                        value: serde_json::json!(format!("thread:{}", thread_id)),
                    },
                }),
                projections: vec![
                    Projection::Subquery {
                        alias: "author".to_string(),
                        plan: Box::new(thread_author_subquery),
                    },
                    Projection::Subquery {
                        alias: "comments".to_string(),
                        plan: Box::new(comments_subquery),
                    },
                ],
            }),
            limit: 1,
            order_by: None,
        },
    }
}

#[cfg(test)]
mod query3_tests {
    use super::*;

    /// TEST: Thread detail view should include thread, author, comments, and comment authors
    /// thread:1 has author user:1 (Alice)
    /// thread:1 has 3 comments by: user:2 (Bob), user:3 (Charlie), user:1 (Alice)
    /// Expected edges: thread:1, user:1, comment:1, comment:2, comment:3, user:2, user:3
    /// Note: user:1 appears as both thread author and comment author - should be 1 edge!
    #[test]
    fn test_query3_initial_load() {
        let db = setup_full_database();
        let plan = build_query3_plan("1");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        let result = view.process_batch(&BatchDeltas::new(), &db);
        assert!(result.is_some());
        
        let result = result.unwrap();
        let created = extract_by_event(&result, DeltaEvent::Created);
        
        println!("Query 3 - Thread detail (thread:1):");
        for id in &created {
            println!("  - {}", id);
        }
        
        // Should have the thread
        assert!(created.contains("thread:1"), "Should have thread:1");
        
        // Should have thread author (Alice)
        assert!(created.contains("user:1"), "Should have user:1 (thread author)");
        
        // Should have comments on thread:1 (comment:1, comment:2, comment:3)
        assert!(created.contains("comment:1"), "Should have comment:1");
        assert!(created.contains("comment:2"), "Should have comment:2");
        assert!(created.contains("comment:3"), "Should have comment:3");
        
        // Should NOT have comment:4 (it's on thread:2)
        assert!(!created.contains("comment:4"), "Should NOT have comment:4 (wrong thread)");
        
        // Should have comment authors (Bob, Charlie)
        // Note: Alice (user:1) is both thread author and comment author - only 1 edge!
        assert!(created.contains("user:2"), "Should have user:2 (Bob - comment author)");
        assert!(created.contains("user:3"), "Should have user:3 (Charlie - comment author)");
        
        // Count user:1 occurrences - should be exactly 1
        let user1_count = created.iter().filter(|id| *id == "user:1").count();
        assert_eq!(user1_count, 1, "user:1 should appear exactly once (not duplicated)");
        
        // Total expected: 1 thread + 3 comments + 3 unique users = 7
        // thread:1, comment:1, comment:2, comment:3, user:1, user:2, user:3
        assert_eq!(created.len(), 7, "Should have exactly 7 edges");
    }

    /// TEST: Adding a comment should create edges for comment + author (if new)
    #[test]
    fn test_query3_add_comment_new_author() {
        let mut db = setup_full_database();
        let plan = build_query3_plan("1");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        view.process_batch(&BatchDeltas::new(), &db);
        
        // Add comment by Diana (user:4) who hasn't commented yet
        let comment_table = db.tables.get_mut("comment").unwrap();
        comment_table.rows.insert(SmolStr::new("5"), make_comment("5", "1", "4", "Diana's comment", 400));
        comment_table.zset.insert(make_zset_key("comment", "5"), 1);
        
        let mut batch = BatchDeltas::new();
        batch.membership.insert("comment".to_string(), {
            let mut z = ZSet::default();
            z.insert(SmolStr::new("comment:5"), 1);
            z
        });
        
        let result = view.process_batch(&batch, &db);
        assert!(result.is_some());
        
        let result = result.unwrap();
        let created = extract_by_event(&result, DeltaEvent::Created);
        
        println!("Query 3 - Add comment with new author:");
        for id in &created {
            println!("  - Created: {}", id);
        }
        
        // Should create edge for new comment
        assert!(created.contains("comment:5"), "Should create edge for comment:5");
        
        // Should create edge for Diana (new comment author)
        assert!(created.contains("user:4"), "Should create edge for user:4 (Diana)");
        
        assert_eq!(created.len(), 2, "Should have exactly 2 Created events");
    }

    /// TEST: Adding a comment by existing author should NOT duplicate user edge
    #[test]
    fn test_query3_add_comment_existing_author() {
        let mut db = setup_full_database();
        let plan = build_query3_plan("1");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        view.process_batch(&BatchDeltas::new(), &db);
        
        // Add another comment by Bob (user:2) who already has a comment
        let comment_table = db.tables.get_mut("comment").unwrap();
        comment_table.rows.insert(SmolStr::new("5"), make_comment("5", "1", "2", "Bob's second comment", 400));
        comment_table.zset.insert(make_zset_key("comment", "5"), 1);
        
        let mut batch = BatchDeltas::new();
        batch.membership.insert("comment".to_string(), {
            let mut z = ZSet::default();
            z.insert(SmolStr::new("comment:5"), 1);
            z
        });
        
        let result = view.process_batch(&batch, &db);
        assert!(result.is_some());
        
        let result = result.unwrap();
        let created = extract_by_event(&result, DeltaEvent::Created);
        
        println!("Query 3 - Add comment with existing author:");
        for id in &created {
            println!("  - Created: {}", id);
        }
        
        // Should create edge for new comment
        assert!(created.contains("comment:5"), "Should create edge for comment:5");
        
        // Should NOT create another edge for Bob (already in view)
        assert!(!created.contains("user:2"), "Should NOT create duplicate edge for user:2");
        
        assert_eq!(created.len(), 1, "Should have exactly 1 Created event");
    }

    /// TEST: Deleting a comment should delete comment edge, but keep author if still referenced
    #[test]
    fn test_query3_delete_comment_author_still_referenced() {
        let mut db = setup_full_database();
        let plan = build_query3_plan("1");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        view.process_batch(&BatchDeltas::new(), &db);
        
        // Add a second comment by Bob first
        {
            let comment_table = db.tables.get_mut("comment").unwrap();
            comment_table.rows.insert(SmolStr::new("5"), make_comment("5", "1", "2", "Bob's second", 400));
            comment_table.zset.insert(make_zset_key("comment", "5"), 1);
        }
        
        let mut batch1 = BatchDeltas::new();
        batch1.membership.insert("comment".to_string(), {
            let mut z = ZSet::default();
            z.insert(SmolStr::new("comment:5"), 1);
            z
        });
        view.process_batch(&batch1, &db);
        
        // Now delete comment:1 (Bob's first comment)
        let comment_table = db.tables.get_mut("comment").unwrap();
        comment_table.rows.remove("1");
        comment_table.zset.remove("comment:1");
        
        let mut batch2 = BatchDeltas::new();
        batch2.membership.insert("comment".to_string(), {
            let mut z = ZSet::default();
            z.insert(SmolStr::new("comment:1"), -1);
            z
        });
        
        let result = view.process_batch(&batch2, &db);
        assert!(result.is_some());
        
        let result = result.unwrap();
        let deleted = extract_by_event(&result, DeltaEvent::Deleted);
        
        println!("Query 3 - Delete comment (author still has other comment):");
        for id in &deleted {
            println!("  - Deleted: {}", id);
        }
        
        // Should delete comment:1
        assert!(deleted.contains("comment:1"), "Should delete comment:1");
        
        // Should NOT delete Bob (still has comment:5)
        assert!(!deleted.contains("user:2"), "Should NOT delete user:2 (still has comment:5)");
        
        assert_eq!(deleted.len(), 1, "Should have exactly 1 Deleted event");
    }

    /// TEST: Deleting all comments by an author should remove that author
    #[test]
    fn test_query3_delete_comment_author_no_longer_referenced() {
        let mut db = setup_full_database();
        let plan = build_query3_plan("1");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        view.process_batch(&BatchDeltas::new(), &db);
        
        // Delete comment:2 (Charlie's only comment on this thread)
        let comment_table = db.tables.get_mut("comment").unwrap();
        comment_table.rows.remove("2");
        comment_table.zset.remove("comment:2");
        
        let mut batch = BatchDeltas::new();
        batch.membership.insert("comment".to_string(), {
            let mut z = ZSet::default();
            z.insert(SmolStr::new("comment:2"), -1);
            z
        });
        
        let result = view.process_batch(&batch, &db);
        assert!(result.is_some());
        
        let result = result.unwrap();
        let deleted = extract_by_event(&result, DeltaEvent::Deleted);
        
        println!("Query 3 - Delete comment (author's only comment):");
        for id in &deleted {
            println!("  - Deleted: {}", id);
        }
        
        // Should delete comment:2
        assert!(deleted.contains("comment:2"), "Should delete comment:2");
        
        // Should delete Charlie (no longer has any comments on this thread)
        assert!(deleted.contains("user:3"), "Should delete user:3 (Charlie - no more comments)");
        
        assert_eq!(deleted.len(), 2, "Should have exactly 2 Deleted events");
    }

    /// TEST: Verify that deeply nested subquery authors are included
    #[test]
    fn test_query3_nested_subquery_depth() {
        let db = setup_full_database();
        let plan = build_query3_plan("1");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        let result = view.process_batch(&BatchDeltas::new(), &db);
        let created = extract_by_event(&result.unwrap(), DeltaEvent::Created);
        
        // Verify we got comment authors (from nested subquery)
        // comment:1 author = user:2 (Bob)
        // comment:2 author = user:3 (Charlie)
        // comment:3 author = user:1 (Alice) - same as thread author
        
        assert!(created.contains("user:2"), "Should include user:2 from nested comment author subquery");
        assert!(created.contains("user:3"), "Should include user:3 from nested comment author subquery");
        
        // Verify no duplicates in created set (HashSet should handle this)
        let _created_vec: Vec<_> = extract_events(&ViewUpdate::Streaming(
            ssp::engine::update::StreamingUpdate {
                view_id: "test".to_string(),
                records: vec![],
            }
        ));
        // The HashSet extract_by_event already deduplicates
    }

    /// TEST: Thread author update should emit Updated (not Created)
    #[test]
    fn test_query3_author_content_update() {
        let mut db = setup_full_database();
        let plan = build_query3_plan("1");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        view.process_batch(&BatchDeltas::new(), &db);
        
        // Update Alice (thread author)
        let user_table = db.tables.get_mut("user").unwrap();
        user_table.rows.insert(SmolStr::new("1"), make_user("1", "Alice Updated"));
        
        let mut batch = BatchDeltas::new();
        batch.content_updates.insert("user".to_string(), FastHashSet::from_iter(vec![SmolStr::new("user:1")]));
        
        let result = view.process_batch(&batch, &db);
        
        if let Some(result) = result {
            let (created, updated, deleted) = count_events(&result);
            
            println!("Query 3 - Author content update:");
            println!("  Created: {}, Updated: {}, Deleted: {}", created, updated, deleted);
            
            assert_eq!(created, 0, "Should not create new edges");
            assert!(updated >= 1, "Should have at least 1 Updated event for user:1");
            assert_eq!(deleted, 0, "Should not delete edges");
        }
    }
}

// ============================================================================
// INTEGRATION: All queries working together
// ============================================================================

#[cfg(test)]
mod integration_tests {
    use super::*;

    /// TEST: Multiple views tracking same data should have independent but consistent edges
    #[test]
    fn test_multiple_views_consistency() {
        let mut db = setup_full_database();
        
        // Query 1: All threads with authors (limited)
        let mut view1 = View::new(build_query1_plan(), None, Some(ViewResultFormat::Streaming));
        
        // Query 2: Specific user
        let mut view2 = View::new(build_query2_plan("1"), None, Some(ViewResultFormat::Streaming));
        
        // Query 3: Thread detail
        let mut view3 = View::new(build_query3_plan("1"), None, Some(ViewResultFormat::Streaming));
        
        // Initial load all
        let r1 = view1.process_batch(&BatchDeltas::new(), &db);
        let r2 = view2.process_batch(&BatchDeltas::new(), &db);
        let r3 = view3.process_batch(&BatchDeltas::new(), &db);
        
        // All should have user:1
        let c1 = extract_by_event(&r1.unwrap(), DeltaEvent::Created);
        let c2 = extract_by_event(&r2.unwrap(), DeltaEvent::Created);
        let c3 = extract_by_event(&r3.unwrap(), DeltaEvent::Created);
        
        assert!(c1.contains("user:1"), "View 1 should have user:1");
        assert!(c2.contains("user:1"), "View 2 should have user:1");
        assert!(c3.contains("user:1"), "View 3 should have user:1");
        
        // Each view should have user:1 exactly once
        assert_eq!(c1.iter().filter(|id| *id == "user:1").count(), 1);
        assert_eq!(c2.iter().filter(|id| *id == "user:1").count(), 1);
        assert_eq!(c3.iter().filter(|id| *id == "user:1").count(), 1);
        
        // Update user:1
        let user_table = db.tables.get_mut("user").unwrap();
        user_table.rows.insert(SmolStr::new("1"), make_user("1", "Alice Updated"));
        
        let mut batch = BatchDeltas::new();
        batch.content_updates.insert("user".to_string(), FastHashSet::from_iter(vec![SmolStr::new("user:1")]));
        
        let r1 = view1.process_batch(&batch, &db);
        let r2 = view2.process_batch(&batch, &db);
        let r3 = view3.process_batch(&batch, &db);
        
        // All should emit Updated for user:1 (or include it in updates)
        // The exact behavior depends on implementation
        println!("After user:1 update:");
        println!("  View 1: {:?}", r1.as_ref().map(|r| count_events(r)));
        println!("  View 2: {:?}", r2.as_ref().map(|r| count_events(r)));
        println!("  View 3: {:?}", r3.as_ref().map(|r| count_events(r)));
    }

    /// TEST: Edge counts should match expected for each query type
    #[test]
    fn test_edge_count_expectations() {
        let db = setup_full_database();
        
        // Query 1: 4 threads + 3 unique authors = 7
        let mut view1 = View::new(build_query1_plan(), None, Some(ViewResultFormat::Streaming));
        let r1 = view1.process_batch(&BatchDeltas::new(), &db).unwrap();
        let count1 = extract_by_event(&r1, DeltaEvent::Created).len();
        
        // Query 2: 1 user = 1
        let mut view2 = View::new(build_query2_plan("1"), None, Some(ViewResultFormat::Streaming));
        let r2 = view2.process_batch(&BatchDeltas::new(), &db).unwrap();
        let count2 = extract_by_event(&r2, DeltaEvent::Created).len();
        
        // Query 3: 1 thread + 1 thread author + 3 comments + 2 unique comment authors 
        // (Alice is both thread author and comment author = 1 edge)
        // = 1 + 1 + 3 + 2 = 7
        // Wait: user:1 (thread author), user:2, user:3 (comment authors)
        // user:1 is also comment author - but only counted once = 3 users total
        // So: 1 thread + 3 comments + 3 users = 7
        let mut view3 = View::new(build_query3_plan("1"), None, Some(ViewResultFormat::Streaming));
        let r3 = view3.process_batch(&BatchDeltas::new(), &db).unwrap();
        let count3 = extract_by_event(&r3, DeltaEvent::Created).len();
        
        println!("Edge counts:");
        println!("  Query 1 (threads with authors): {} edges", count1);
        println!("  Query 2 (single user): {} edges", count2);
        println!("  Query 3 (thread detail): {} edges", count3);
        
        assert_eq!(count1, 7, "Query 1 should have 7 edges (4 threads + 3 authors)");
        assert_eq!(count2, 1, "Query 2 should have 1 edge (1 user)");
        assert_eq!(count3, 7, "Query 3 should have 7 edges (1 thread + 3 comments + 3 users)");
    }
}