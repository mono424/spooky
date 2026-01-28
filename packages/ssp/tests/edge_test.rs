//! Ultimate Test Suite for SSP Edge & Delta System
//!
//! This test suite covers all identified issues and edge cases:
//! - Membership model correctness
//! - Weight normalization
//! - Fast path vs batch path consistency
//! - Subquery weight handling
//! - Deserialization
//! - Content updates vs membership changes
//! - Edge event generation
//!
//! Run with: cargo test --package ssp --lib -- engine::view::ultimate_tests --nocapture

#![allow(unused)]
use ssp::engine::circuit::{Circuit, Database, Table};
use ssp::engine::operators::{Operator, Predicate, Projection, OrderSpec};
use ssp::engine::types::{
    BatchDeltas, Delta, FastMap, Path, SpookyValue, ZSet,
    ZSetMembershipOps,
    make_zset_key, parse_zset_key,
};
use ssp::engine::update::{DeltaEvent, ViewResultFormat, ViewUpdate};
use ssp::engine::view::{QueryPlan, View};
use smol_str::SmolStr;


// ============================================================================
// TEST HELPERS
// ============================================================================

fn make_zset(items: &[(&str, i64)]) -> ZSet {
    items.iter().map(|(k, w)| (SmolStr::new(*k), *w)).collect()
}

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

fn make_comment(id: &str, thread_id: &str, author_id: &str, text: &str) -> SpookyValue {
    let mut map = FastMap::default();
    map.insert(SmolStr::new("id"), SpookyValue::Str(SmolStr::new(format!("comment:{}", id))));
    map.insert(SmolStr::new("thread"), SpookyValue::Str(SmolStr::new(format!("thread:{}", thread_id))));
    map.insert(SmolStr::new("author"), SpookyValue::Str(SmolStr::new(format!("user:{}", author_id))));
    map.insert(SmolStr::new("text"), SpookyValue::Str(SmolStr::new(text)));
    SpookyValue::Object(map)
}

fn setup_db_with_users(users: &[(&str, &str)]) -> Database {
    let mut db = Database::new();
    let table = db.ensure_table("user");
    
    for (id, name) in users {
        table.rows.insert(SmolStr::new(*id), make_user(id, name));
        table.zset.insert(make_zset_key("user", id), 1);
    }
    
    db
}

fn setup_db_with_threads_and_users(
    users: &[(&str, &str)],
    threads: &[(&str, &str, &str)], // (id, author_id, title)
) -> Database {
    let mut db = Database::new();
    
    // Add users
    let user_table = db.ensure_table("user");
    for (id, name) in users {
        user_table.rows.insert(SmolStr::new(*id), make_user(id, name));
        user_table.zset.insert(make_zset_key("user", id), 1);
    }
    
    // Add threads
    let thread_table = db.ensure_table("thread");
    for (id, author_id, title) in threads {
        thread_table.rows.insert(SmolStr::new(*id), make_thread(id, author_id, title));
        thread_table.zset.insert(make_zset_key("thread", id), 1);
    }
    
    db
}

fn simple_scan_plan(table: &str) -> QueryPlan {
    QueryPlan {
        id: format!("scan_{}", table),
        root: Operator::Scan { table: table.to_string() },
    }
}

fn filter_by_id_plan(table: &str, id_value: &str) -> QueryPlan {
    QueryPlan {
        id: format!("filter_{}_{}", table, id_value),
        root: Operator::Filter {
            input: Box::new(Operator::Scan { table: table.to_string() }),
            predicate: Predicate::Eq {
                field: Path::new("id"),
                value: serde_json::json!(id_value),
            },
        },
    }
}

fn thread_with_author_subquery_plan() -> QueryPlan {
    QueryPlan {
        id: "thread_with_author".to_string(),
        root: Operator::Project {
            input: Box::new(Operator::Scan { table: "thread".to_string() }),
            projections: vec![
                Projection::Subquery {
                    alias: "author".to_string(),
                    plan: Box::new(Operator::Filter {
                        input: Box::new(Operator::Scan { table: "user".to_string() }),
                        predicate: Predicate::Eq {
                            field: Path::new("id"),
                            value: serde_json::json!({"$param": "parent.author"}),
                        },
                    }),
                },
            ],
        },
    }
}

fn limit_plan(table: &str, limit: usize) -> QueryPlan {
    QueryPlan {
        id: format!("limit_{}_{}", table, limit),
        root: Operator::Limit {
            input: Box::new(Operator::Scan { table: table.to_string() }),
            limit,
            order_by: None,
        },
    }
}

/// Extract delta events from ViewUpdate
fn extract_events(update: &ViewUpdate) -> Vec<(String, DeltaEvent)> {
    match update {
        ViewUpdate::Streaming(s) => {
            s.records.iter()
                .map(|r| (r.id.to_string(), r.event.clone()))
                .collect()
        }
        _ => vec![],
    }
}

/// Count events by type
fn count_events(update: &ViewUpdate) -> (usize, usize, usize) {
    let events = extract_events(update);
    let created = events.iter().filter(|(_, e)| matches!(e, DeltaEvent::Created)).count();
    let updated = events.iter().filter(|(_, e)| matches!(e, DeltaEvent::Updated)).count();
    let deleted = events.iter().filter(|(_, e)| matches!(e, DeltaEvent::Deleted)).count();
    (created, updated, deleted)
}

// ============================================================================
// PART 1: MEMBERSHIP MODEL TESTS
// ============================================================================

#[cfg(test)]
mod membership_model_tests {
    use super::*;

    /// TEST: Cache weights should always be 1 (membership) not accumulated
    #[test]
    fn test_cache_weights_normalized_to_one() {
        let db = setup_db_with_users(&[("1", "Alice"), ("2", "Bob")]);
        let plan = simple_scan_plan("user");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // First run
        view.process_batch(&BatchDeltas::new(), &db);
        
        // All weights should be 1
        for (key, &weight) in &view.cache {
            assert_eq!(weight, 1, "Weight for {} should be 1, got {}", key, weight);
        }
    }

    /// TEST: Re-adding same record should keep weight at 1, not increment
    #[test]
    fn test_readd_record_keeps_weight_one() {
        let mut db = setup_db_with_users(&[("1", "Alice")]);
        let plan = simple_scan_plan("user");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // First add
        let delta1 = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:1"),
            weight: 1,
            content_changed: false,
        };
        view.process_delta(&delta1, &db);
        assert_eq!(view.cache.get("user:1"), Some(&1));
        
        // Re-add same record (simulating duplicate event)
        let delta2 = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:1"),
            weight: 1,
            content_changed: false,
        };
        view.process_delta(&delta2, &db);
        
        // Weight should still be 1, NOT 2
        assert_eq!(
            view.cache.get("user:1"), 
            Some(&1), 
            "Weight should remain 1 after re-add, not increment"
        );
    }

    /// TEST: Membership delta correctly identifies additions and removals
    #[test]
    fn test_membership_diff_correctness() {
        let old = make_zset(&[("a", 1), ("b", 1), ("c", 1)]);
        let new = make_zset(&[("b", 1), ("c", 1), ("d", 1)]);
        
        let (additions, removals) = old.membership_diff(&new);
        
        assert_eq!(additions.len(), 1);
        assert_eq!(removals.len(), 1);
        assert!(additions.contains(&SmolStr::new("d")), "d should be added");
        assert!(removals.contains(&SmolStr::new("a")), "a should be removed");
    }

    /// TEST: Weight changes (1->2, 2->1) should NOT be membership changes
    #[test]
    fn test_weight_change_not_membership_change() {
        let old = make_zset(&[("a", 1)]);
        let new = make_zset(&[("a", 5)]); // Weight increased but still present
        
        let (additions, removals) = old.membership_diff(&new);
        
        assert!(additions.is_empty(), "Weight increase should not be an addition");
        assert!(removals.is_empty(), "Weight change should not be a removal");
    }

    /// TEST: Zero weight means removed from membership
    #[test]
    fn test_zero_weight_is_not_member() {
        let zset = make_zset(&[("a", 1), ("b", 0), ("c", -1)]);
        
        assert!(zset.is_member("a"), "a with weight 1 should be member");
        assert!(!zset.is_member("b"), "b with weight 0 should NOT be member");
        assert!(!zset.is_member("c"), "c with weight -1 should NOT be member");
    }

    /// TEST: apply_membership_delta normalizes all weights to 1
    #[test]
    fn test_apply_membership_delta_normalizes() {
        let mut cache = make_zset(&[("a", 1), ("b", 1)]);
        let delta = make_zset(&[
            ("a", 5),   // +5 -> would be 6, should normalize to 1
            ("b", -1),  // -1 -> becomes 0, should be removed
            ("c", 3),   // +3 -> new, should normalize to 1
        ]);
        
        cache.apply_membership_delta(&delta);
        
        assert_eq!(cache.get("a"), Some(&1), "a should be normalized to 1");
        assert!(!cache.contains_key("b"), "b should be removed (weight 0)");
        assert_eq!(cache.get("c"), Some(&1), "c should be normalized to 1");
    }

    /// TEST: normalize_to_membership cleans up all non-1 weights
    #[test]
    fn test_normalize_to_membership() {
        let mut zset = make_zset(&[
            ("a", 1),
            ("b", 5),   // Should become 1
            ("c", 0),   // Should be removed
            ("d", -2),  // Should be removed
        ]);
        
        zset.normalize_to_membership();
        
        assert_eq!(zset.get("a"), Some(&1));
        assert_eq!(zset.get("b"), Some(&1));
        assert!(!zset.contains_key("c"));
        assert!(!zset.contains_key("d"));
        assert_eq!(zset.len(), 2);
    }
}

// ============================================================================
// PART 2: FAST PATH VS BATCH PATH CONSISTENCY
// ============================================================================

#[cfg(test)]
mod fast_path_tests {
    use super::*;

    /// TEST: Fast path and batch path should produce identical results
    #[test]
    fn test_fast_path_batch_path_equivalence() {
        let db = setup_db_with_users(&[("1", "Alice")]);
        
        // View 1: Use fast path (simple Scan)
        let plan1 = simple_scan_plan("user");
        let mut view_fast = View::new(plan1, None, Some(ViewResultFormat::Streaming));
        
        // View 2: Force batch path by using same plan
        let plan2 = simple_scan_plan("user");
        let mut view_batch = View::new(plan2, None, Some(ViewResultFormat::Streaming));
        
        // Fast path: process_delta
        let delta = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:1"),
            weight: 1,
            content_changed: false,
        };
        let result_fast = view_fast.process_delta(&delta, &db);
        
        // Batch path: process_batch
        let mut batch = BatchDeltas::new();
        batch.membership.insert("user".to_string(), make_zset(&[("user:1", 1)]));
        let result_batch = view_batch.process_batch(&batch, &db);
        
        // Both should have same cache state
        assert_eq!(view_fast.cache, view_batch.cache, "Cache should match");
        
        // Both should produce Created event
        assert!(result_fast.is_some());
        assert!(result_batch.is_some());
        
        let (created_fast, _, _) = count_events(&result_fast.unwrap());
        let (created_batch, _, _) = count_events(&result_batch.unwrap());
        assert_eq!(created_fast, created_batch, "Should have same number of Created events");
    }

    /// TEST: Fast path delete should work correctly
    #[test]
    fn test_fast_path_delete() {
        let db = setup_db_with_users(&[("1", "Alice")]);
        let plan = simple_scan_plan("user");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Add first
        let add_delta = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:1"),
            weight: 1,
            content_changed: false,
        };
        view.process_delta(&add_delta, &db);
        assert!(view.cache.is_member("user:1"));
        
        // Delete
        let del_delta = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:1"),
            weight: -1,
            content_changed: false,
        };
        let result = view.process_delta(&del_delta, &db);
        
        // Should be removed from cache
        assert!(!view.cache.contains_key("user:1"), "Record should be removed from cache");
        
        // Should emit Deleted event
        assert!(result.is_some());
        let (_, _, deleted) = count_events(&result.unwrap());
        assert_eq!(deleted, 1, "Should emit 1 Deleted event");
    }

    /// TEST: Fast path filter should correctly filter records
    #[test]
    fn test_fast_path_filter() {
        let db = setup_db_with_users(&[("1", "Alice"), ("2", "Bob")]);
        let plan = filter_by_id_plan("user", "user:1");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Try to add user:2 (should be filtered out)
        let delta2 = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:2"),
            weight: 1,
            content_changed: false,
        };
        let result = view.process_delta(&delta2, &db);
        
        // Should not be added
        assert!(result.is_none() || view.cache.is_empty());
        
        // Add user:1 (should pass filter)
        let delta1 = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:1"),
            weight: 1,
            content_changed: false,
        };
        let result = view.process_delta(&delta1, &db);
        
        // Should be added
        assert!(result.is_some());
        assert!(view.cache.is_member("user:1"));
    }

    /// TEST: Fast path should not be used for complex views
    #[test]
    fn test_fast_path_not_used_for_complex_views() {
        let db = setup_db_with_threads_and_users(
            &[("1", "Alice")],
            &[("1", "1", "Thread 1")],
        );
        
        // View with subquery - should use batch path
        let plan = thread_with_author_subquery_plan();
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Should NOT use fast path (has subqueries)
        assert!(!view.is_simple_scan);
        assert!(!view.is_simple_filter);
        
        // Process should still work via batch path
        let result = view.process_batch(&BatchDeltas::new(), &db);
        assert!(result.is_some());
    }
}

// ============================================================================
// PART 3: SUBQUERY WEIGHT HANDLING (THE ORIGINAL BUG)
// ============================================================================

#[cfg(test)]
mod subquery_tests {
    use super::*;

    /// TEST: User referenced by multiple threads should have weight 1, not N
    /// This was the original bug - user was appearing as "Created" multiple times
    #[test]
    fn test_user_referenced_by_multiple_threads_weight_one() {
        let db = setup_db_with_threads_and_users(
            &[("1", "Alice")],  // 1 user
            &[
                ("1", "1", "Thread 1"),  // Thread 1 by Alice
                ("2", "1", "Thread 2"),  // Thread 2 by Alice
                ("3", "1", "Thread 3"),  // Thread 3 by Alice
            ],
        );
        
        let plan = thread_with_author_subquery_plan();
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Process all records
        let result = view.process_batch(&BatchDeltas::new(), &db);
        
        // Check user weight in cache - MUST be 1, not 3!
        let user_weight = view.cache.get("user:1").copied().unwrap_or(0);
        assert_eq!(
            user_weight, 1,
            "User referenced by 3 threads should have weight 1, not {}",
            user_weight
        );
        
        // Check events - user should appear exactly ONCE as Created
        let result = result.unwrap();
        let events = extract_events(&result);
        let user_created_count = events.iter()
            .filter(|(id, event)| id == "user:1" && matches!(event, DeltaEvent::Created))
            .count();
        
        assert_eq!(
            user_created_count, 1,
            "User should have exactly 1 Created event, not {}",
            user_created_count
        );
    }

    /// TEST: Deleting one thread should NOT delete the user (still referenced)
    #[test]
    fn test_delete_one_thread_user_stays() {
        let mut db = setup_db_with_threads_and_users(
            &[("1", "Alice")],
            &[
                ("1", "1", "Thread 1"),
                ("2", "1", "Thread 2"),
            ],
        );
        
        let plan = thread_with_author_subquery_plan();
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        view.process_batch(&BatchDeltas::new(), &db);
        assert!(view.cache.is_member("user:1"));
        assert!(view.cache.is_member("thread:1"));
        assert!(view.cache.is_member("thread:2"));
        
        // Delete thread:1 from database
        let thread_table = db.tables.get_mut("thread").unwrap();
        thread_table.rows.remove("1");
        thread_table.zset.remove("thread:1");
        
        // Process deletion
        let mut batch = BatchDeltas::new();
        batch.membership.insert("thread".to_string(), make_zset(&[("thread:1", -1)]));
        let result = view.process_batch(&batch, &db);
        
        // User should STILL be in cache (still referenced by thread:2)
        assert!(
            view.cache.is_member("user:1"),
            "User should still be in view (referenced by thread:2)"
        );
        
        // Thread:1 should be removed
        assert!(!view.cache.is_member("thread:1"));
        
        // Check events
        let result = result.unwrap();
        let events = extract_events(&result);
        
        // Should have Deleted for thread:1
        assert!(
            events.iter().any(|(id, e)| id == "thread:1" && matches!(e, DeltaEvent::Deleted)),
            "thread:1 should be Deleted"
        );
        
        // Should NOT have Deleted for user:1
        assert!(
            !events.iter().any(|(id, e)| id == "user:1" && matches!(e, DeltaEvent::Deleted)),
            "user:1 should NOT be Deleted (still referenced)"
        );
    }

    /// TEST: Deleting ALL threads should delete the user
    #[test]
    fn test_delete_all_threads_user_removed() {
        let mut db = setup_db_with_threads_and_users(
            &[("1", "Alice")],
            &[("1", "1", "Thread 1")],
        );
        
        let plan = thread_with_author_subquery_plan();
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        view.process_batch(&BatchDeltas::new(), &db);
        assert!(view.cache.is_member("user:1"));
        
        // Delete the only thread
        let thread_table = db.tables.get_mut("thread").unwrap();
        thread_table.rows.remove("1");
        thread_table.zset.remove("thread:1");
        
        // Process deletion
        let mut batch = BatchDeltas::new();
        batch.membership.insert("thread".to_string(), make_zset(&[("thread:1", -1)]));
        let result = view.process_batch(&batch, &db);
        
        // User should be REMOVED (no longer referenced)
        assert!(
            !view.cache.is_member("user:1"),
            "User should be removed (no threads reference it)"
        );
        
        // Check events
        let result = result.unwrap();
        let events = extract_events(&result);
        
        // Should have Deleted for both
        assert!(
            events.iter().any(|(id, e)| id == "thread:1" && matches!(e, DeltaEvent::Deleted)),
            "thread:1 should be Deleted"
        );
        assert!(
            events.iter().any(|(id, e)| id == "user:1" && matches!(e, DeltaEvent::Deleted)),
            "user:1 should be Deleted (no longer referenced)"
        );
    }

    /// TEST: Adding new thread with same author should NOT create user again
    #[test]
    fn test_add_thread_same_author_no_duplicate_user() {
        let mut db = setup_db_with_threads_and_users(
            &[("1", "Alice")],
            &[("1", "1", "Thread 1")],
        );
        
        let plan = thread_with_author_subquery_plan();
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        view.process_batch(&BatchDeltas::new(), &db);
        
        // Add new thread with same author
        let thread_table = db.tables.get_mut("thread").unwrap();
        thread_table.rows.insert(SmolStr::new("2"), make_thread("2", "1", "Thread 2"));
        thread_table.zset.insert(SmolStr::new("thread:2"), 1);
        
        // Process addition
        let mut batch = BatchDeltas::new();
        batch.membership.insert("thread".to_string(), make_zset(&[("thread:2", 1)]));
        let result = view.process_batch(&batch, &db);
        
        // User weight should still be 1
        assert_eq!(view.cache.get("user:1"), Some(&1));
        
        // Check events
        let result = result.unwrap();
        let events = extract_events(&result);
        
        // thread:2 should be Created
        assert!(
            events.iter().any(|(id, e)| id == "thread:2" && matches!(e, DeltaEvent::Created)),
            "thread:2 should be Created"
        );
        
        // user:1 should NOT be Created again
        assert!(
            !events.iter().any(|(id, e)| id == "user:1" && matches!(e, DeltaEvent::Created)),
            "user:1 should NOT be Created again (already in view)"
        );
    }
}

// ============================================================================
// PART 4: CONTENT UPDATES VS MEMBERSHIP CHANGES
// ============================================================================

#[cfg(test)]
mod content_update_tests {
    use super::*;

    /// TEST: Content update should emit Updated event, not Created
    #[test]
    fn test_content_update_emits_updated() {
        let mut db = setup_db_with_users(&[("1", "Alice")]);
        let plan = simple_scan_plan("user");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        view.process_batch(&BatchDeltas::new(), &db);
        
        // Update user content
        let user_table = db.tables.get_mut("user").unwrap();
        user_table.rows.insert(SmolStr::new("1"), make_user("1", "Alice Updated"));
        
        // Process content update (weight=0, content_changed=true)
        let delta = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:1"),
            weight: 0,
            content_changed: true,
        };
        let result = view.process_delta(&delta, &db);
        
        // Should emit Updated, not Created
        let result = result.unwrap();
        let events = extract_events(&result);
        
        assert!(
            events.iter().any(|(id, e)| id == "user:1" && matches!(e, DeltaEvent::Updated)),
            "Should emit Updated event"
        );
        assert!(
            !events.iter().any(|(id, e)| id == "user:1" && matches!(e, DeltaEvent::Created)),
            "Should NOT emit Created event"
        );
    }

    /// TEST: Content update for record not in view should be ignored
    #[test]
    fn test_content_update_for_non_member_ignored() {
        let db = setup_db_with_users(&[("1", "Alice")]);
        let plan = filter_by_id_plan("user", "user:999"); // Filters to non-existent user
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load (empty view)
        view.process_batch(&BatchDeltas::new(), &db);
        assert!(view.cache.is_empty());
        
        // Content update for user:1 (not in view)
        let delta = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:1"),
            weight: 0,
            content_changed: true,
        };
        let result = view.process_delta(&delta, &db);
        
        // Should be ignored
        assert!(result.is_none(), "Content update for non-member should be ignored");
    }

    /// TEST: Content update that makes record no longer match filter should remove it
    #[test]
    fn test_content_update_filter_mismatch_removes() {
        let mut db = Database::new();
        let user_table = db.ensure_table("user");
        
        // Add user with status = "active"
        let mut user_data = FastMap::default();
        user_data.insert(SmolStr::new("id"), SpookyValue::Str(SmolStr::new("user:1")));
        user_data.insert(SmolStr::new("status"), SpookyValue::Str(SmolStr::new("active")));
        user_table.rows.insert(SmolStr::new("1"), SpookyValue::Object(user_data));
        user_table.zset.insert(SmolStr::new("user:1"), 1);
        
        // View: WHERE status = "active"
        let plan = QueryPlan {
            id: "active_users".to_string(),
            root: Operator::Filter {
                input: Box::new(Operator::Scan { table: "user".to_string() }),
                predicate: Predicate::Eq {
                    field: Path::new("status"),
                    value: serde_json::json!("active"),
                },
            },
        };
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load
        view.process_batch(&BatchDeltas::new(), &db);
        assert!(view.cache.is_member("user:1"));
        
        // Update user to status = "inactive"
        let user_table = db.tables.get_mut("user").unwrap();
        let mut updated_user = FastMap::default();
        updated_user.insert(SmolStr::new("id"), SpookyValue::Str(SmolStr::new("user:1")));
        updated_user.insert(SmolStr::new("status"), SpookyValue::Str(SmolStr::new("inactive")));
        user_table.rows.insert(SmolStr::new("1"), SpookyValue::Object(updated_user));
        
        // Process content update
        let delta = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:1"),
            weight: 0,
            content_changed: true,
        };
        let result = view.process_delta(&delta, &db);
        
        // User should be removed from view
        assert!(
            !view.cache.is_member("user:1"),
            "User should be removed (no longer matches filter)"
        );
        
        // Should emit Deleted
        let result = result.unwrap();
        let events = extract_events(&result);
        assert!(
            events.iter().any(|(id, e)| id == "user:1" && matches!(e, DeltaEvent::Deleted)),
            "Should emit Deleted event"
        );
    }
}

// ============================================================================
// PART 5: DESERIALIZATION TESTS
// ============================================================================

#[cfg(test)]
mod deserialization_tests {
    use super::*;

    /// TEST: Cached flags should be restored after deserialization
    #[test]
    fn test_cached_flags_restored_after_deserialize() {
        let plan = QueryPlan {
            id: "test".to_string(),
            root: Operator::Filter {
                input: Box::new(Operator::Scan { table: "user".to_string() }),
                predicate: Predicate::Eq {
                    field: Path::new("id"),
                    value: serde_json::json!("user:1"),
                },
            },
        };
        let original = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Verify original flags
        assert!(original.is_simple_filter);
        assert!(!original.is_simple_scan);
        assert!(!original.referenced_tables_cached.is_empty());
        
        // Serialize
        let json = serde_json::to_string(&original).unwrap();
        
        // Deserialize
        let mut loaded: View = serde_json::from_str(&json).unwrap();
        
        // Before initialization - flags are default
        assert!(!loaded.is_simple_filter);
        assert!(loaded.referenced_tables_cached.is_empty());
        
        // Initialize
        loaded.initialize_after_deserialize();
        
        // After initialization - flags match original
        assert_eq!(loaded.is_simple_filter, original.is_simple_filter);
        assert_eq!(loaded.is_simple_scan, original.is_simple_scan);
        assert_eq!(loaded.referenced_tables_cached, original.referenced_tables_cached);
    }

    /// TEST: View should work correctly after deserialization + initialization
    #[test]
    fn test_view_works_after_deserialize() {
        let db = setup_db_with_users(&[("1", "Alice")]);
        let plan = simple_scan_plan("user");
        let mut original = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Process some data
        original.process_batch(&BatchDeltas::new(), &db);
        
        // Serialize
        let json = serde_json::to_string(&original).unwrap();
        
        // Deserialize and initialize
        let mut loaded: View = serde_json::from_str(&json).unwrap();
        loaded.initialize_after_deserialize();
        
        // Add new record
        let mut db2 = db.clone();
        let user_table = db2.tables.get_mut("user").unwrap();
        user_table.rows.insert(SmolStr::new("2"), make_user("2", "Bob"));
        user_table.zset.insert(SmolStr::new("user:2"), 1);
        
        // Should process correctly
        let delta = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:2"),
            weight: 1,
            content_changed: false,
        };
        let result = loaded.process_delta(&delta, &db2);
        
        // Should work
        assert!(result.is_some());
        assert!(loaded.cache.is_member("user:2"));
    }

    /// TEST: Cache should be preserved across serialization
    #[test]
    fn test_cache_preserved_across_serialization() {
        let db = setup_db_with_users(&[("1", "Alice"), ("2", "Bob")]);
        let plan = simple_scan_plan("user");
        let mut original = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Build cache
        original.process_batch(&BatchDeltas::new(), &db);
        let original_cache = original.cache.clone();
        
        // Serialize and deserialize
        let json = serde_json::to_string(&original).unwrap();
        let loaded: View = serde_json::from_str(&json).unwrap();
        
        // Cache should be preserved
        assert_eq!(loaded.cache, original_cache);
    }
}

// ============================================================================
// PART 6: FIRST RUN / INITIAL LOAD TESTS
// ============================================================================

#[cfg(test)]
mod first_run_tests {
    use super::*;

    /// TEST: First run should emit all records as Created
    #[test]
    fn test_first_run_emits_all_created() {
        let db = setup_db_with_users(&[("1", "Alice"), ("2", "Bob"), ("3", "Charlie")]);
        let plan = simple_scan_plan("user");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // First run
        let result = view.process_batch(&BatchDeltas::new(), &db);
        
        let result = result.unwrap();
        let (created, updated, deleted) = count_events(&result);
        
        assert_eq!(created, 3, "All 3 users should be Created");
        assert_eq!(updated, 0, "No updates on first run");
        assert_eq!(deleted, 0, "No deletions on first run");
    }

    /// TEST: Second run with no changes should emit nothing
    #[test]
    fn test_second_run_no_changes() {
        let db = setup_db_with_users(&[("1", "Alice")]);
        let plan = simple_scan_plan("user");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // First run
        view.process_batch(&BatchDeltas::new(), &db);
        
        // Second run with same data
        let result = view.process_batch(&BatchDeltas::new(), &db);
        
        // Should return None (no changes)
        assert!(result.is_none(), "No changes should return None");
    }

    /// TEST: First run with subqueries should emit unique records only
    #[test]
    fn test_first_run_subquery_unique_records() {
        let db = setup_db_with_threads_and_users(
            &[("1", "Alice")],
            &[
                ("1", "1", "Thread 1"),
                ("2", "1", "Thread 2"),
            ],
        );
        
        let plan = thread_with_author_subquery_plan();
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // First run
        let result = view.process_batch(&BatchDeltas::new(), &db);
        let result = result.unwrap();
        
        let events = extract_events(&result);
        
        // Count unique Created events
        let created_ids: std::collections::HashSet<_> = events.iter()
            .filter(|(_, e)| matches!(e, DeltaEvent::Created))
            .map(|(id, _)| id.clone())
            .collect();
        
        // Should have: thread:1, thread:2, user:1 (NOT user:1 twice!)
        assert!(created_ids.contains("thread:1"));
        assert!(created_ids.contains("thread:2"));
        assert!(created_ids.contains("user:1"));
        assert_eq!(created_ids.len(), 3, "Should have exactly 3 unique Created events");
    }
}

// ============================================================================
// PART 7: LIMIT OPERATOR TESTS
// ============================================================================

#[cfg(test)]
mod limit_tests {
    use super::*;

    /// TEST: Limit should correctly restrict results
    #[test]
    fn test_limit_restricts_results() {
        let db = setup_db_with_users(&[
            ("1", "Alice"),
            ("2", "Bob"),
            ("3", "Charlie"),
            ("4", "David"),
        ]);
        
        let plan = limit_plan("user", 2);
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        view.process_batch(&BatchDeltas::new(), &db);
        
        assert_eq!(view.cache.len(), 2, "Should have exactly 2 records");
    }

    /// TEST: Adding record might push another out of limit
    #[test]
    fn test_limit_addition_may_remove_another() {
        let mut db = setup_db_with_users(&[
            ("1", "Alice"),
            ("2", "Bob"),
        ]);
        
        // Limit 2, ordered by id
        let plan = QueryPlan {
            id: "limit_2_ordered".to_string(),
            root: Operator::Limit {
                input: Box::new(Operator::Scan { table: "user".to_string() }),
                limit: 2,
                order_by: Some(vec![OrderSpec {
                    field: Path::new("id"),
                    direction: "ASC".to_string(),
                }]),
            },
        };
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Initial load: user:1, user:2
        view.process_batch(&BatchDeltas::new(), &db);
        
        // Add user:0 (should be first in order)
        let user_table = db.tables.get_mut("user").unwrap();
        user_table.rows.insert(SmolStr::new("0"), make_user("0", "Zara"));
        user_table.zset.insert(SmolStr::new("user:0"), 1);
        
        let mut batch = BatchDeltas::new();
        batch.membership.insert("user".to_string(), make_zset(&[("user:0", 1)]));
        let result = view.process_batch(&batch, &db);
        
        // With LIMIT 2 and ordered by id ASC:
        // user:0 and user:1 should be in view
        // user:2 should be pushed out
        assert_eq!(view.cache.len(), 2);
        
        let result = result.unwrap();
        let events = extract_events(&result);
        
        // user:0 should be Created
        assert!(
            events.iter().any(|(id, e)| id == "user:0" && matches!(e, DeltaEvent::Created)),
            "user:0 should be Created"
        );
        
        // user:2 should be Deleted (pushed out)
        assert!(
            events.iter().any(|(id, e)| id == "user:2" && matches!(e, DeltaEvent::Deleted)),
            "user:2 should be Deleted (pushed out of limit)"
        );
    }
}

// ============================================================================
// PART 8: EDGE EVENT GENERATION TESTS
// ============================================================================

#[cfg(test)]
mod edge_event_tests {
    use super::*;

    /// TEST: Complete lifecycle - Created, Updated, Deleted
    #[test]
    fn test_complete_lifecycle() {
        let mut db = setup_db_with_users(&[("1", "Alice")]);
        let plan = simple_scan_plan("user");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // 1. CREATE
        let create_delta = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:1"),
            weight: 1,
            content_changed: false,
        };
        let result = view.process_delta(&create_delta, &db);
        assert!(result.is_some());
        let (created, _, _) = count_events(&result.unwrap());
        assert_eq!(created, 1, "Step 1: Should emit Created");
        
        // 2. UPDATE
        let user_table = db.tables.get_mut("user").unwrap();
        user_table.rows.insert(SmolStr::new("1"), make_user("1", "Alice Updated"));
        
        let update_delta = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:1"),
            weight: 0,
            content_changed: true,
        };
        let result = view.process_delta(&update_delta, &db);
        assert!(result.is_some());
        let (_, updated, _) = count_events(&result.unwrap());
        assert_eq!(updated, 1, "Step 2: Should emit Updated");
        
        // 3. DELETE
        let delete_delta = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:1"),
            weight: -1,
            content_changed: false,
        };
        let result = view.process_delta(&delete_delta, &db);
        assert!(result.is_some());
        let (_, _, deleted) = count_events(&result.unwrap());
        assert_eq!(deleted, 1, "Step 3: Should emit Deleted");
        
        // Cache should be empty
        assert!(view.cache.is_empty());
    }

    /// TEST: Multiple views should each get their own events
    #[test]
    fn test_multiple_views_independent_events() {
        let db = setup_db_with_users(&[("1", "Alice")]);
        
        // View 1: All users
        let plan1 = simple_scan_plan("user");
        let mut view1 = View::new(plan1, None, Some(ViewResultFormat::Streaming));
        
        // View 2: Filter to user:1
        let plan2 = filter_by_id_plan("user", "user:1");
        let mut view2 = View::new(plan2, None, Some(ViewResultFormat::Streaming));
        
        // Both should see user:1 on first run
        let result1 = view1.process_batch(&BatchDeltas::new(), &db);
        let result2 = view2.process_batch(&BatchDeltas::new(), &db);
        
        assert!(result1.is_some());
        assert!(result2.is_some());
        
        let (created1, _, _) = count_events(&result1.unwrap());
        let (created2, _, _) = count_events(&result2.unwrap());
        
        assert_eq!(created1, 1);
        assert_eq!(created2, 1);
    }

    /// TEST: Event IDs should match cache keys format
    #[test]
    fn test_event_ids_format() {
        let db = setup_db_with_users(&[("1", "Alice")]);
        let plan = simple_scan_plan("user");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        let result = view.process_batch(&BatchDeltas::new(), &db);
        let result = result.unwrap();
        
        let events = extract_events(&result);
        
        // Event IDs should be in "table:id" format
        for (id, _) in &events {
            assert!(
                id.contains(':'),
                "Event ID '{}' should be in 'table:id' format",
                id
            );
        }
    }
}

// ============================================================================
// PART 9: ZSet KEY HANDLING TESTS
// ============================================================================

#[cfg(test)]
mod zset_key_tests {
    use super::*;

    /// TEST: make_zset_key should prevent double prefixing
    #[test]
    fn test_make_zset_key_no_double_prefix() {
        // Normal case
        assert_eq!(make_zset_key("user", "123").as_str(), "user:123");
        
        // Already prefixed - should strip and re-add
        assert_eq!(make_zset_key("user", "user:123").as_str(), "user:123");
        
        // Different prefix - complex case, depends on implementation
        let result = make_zset_key("thread", "user:123");
        // Should either be "thread:user:123" or "thread:123"
        assert!(result.starts_with("thread:"));
    }

    /// TEST: parse_zset_key should correctly split
    #[test]
    fn test_parse_zset_key() {
        assert_eq!(parse_zset_key("user:123"), Some(("user", "123")));
        assert_eq!(parse_zset_key("thread:abc"), Some(("thread", "abc")));
        assert_eq!(parse_zset_key("nocolon"), None);
    }
}

// ============================================================================
// PART 10: STRESS / EDGE CASE TESTS
// ============================================================================

#[cfg(test)]
mod stress_tests {
    use super::*;

    /// TEST: Large batch should handle correctly
    #[test]
    fn test_large_batch() {
        let mut db = Database::new();
        let user_table = db.ensure_table("user");
        
        // Add 1000 users
        for i in 0..1000 {
            let id = format!("{}", i);
            user_table.rows.insert(SmolStr::new(&id), make_user(&id, &format!("User {}", i)));
            user_table.zset.insert(make_zset_key("user", &id), 1);
        }
        
        let plan = simple_scan_plan("user");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        let result = view.process_batch(&BatchDeltas::new(), &db);
        
        assert!(result.is_some());
        assert_eq!(view.cache.len(), 1000);
        
        let (created, _, _) = count_events(&result.unwrap());
        assert_eq!(created, 1000);
    }

    /// TEST: Empty database should handle correctly
    #[test]
    fn test_empty_database() {
        let db = Database::new();
        let plan = simple_scan_plan("user");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        let result = view.process_batch(&BatchDeltas::new(), &db);
        
        // Empty result, not error
        assert!(result.is_none() || view.cache.is_empty());
    }

    /// TEST: Rapid add/delete cycles
    #[test]
    fn test_rapid_add_delete() {
        let mut db = setup_db_with_users(&[("1", "Alice")]);
        let plan = simple_scan_plan("user");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        for _ in 0..10 {
            // Add
            let add_delta = Delta {
                table: SmolStr::new("user"),
                key: SmolStr::new("user:1"),
                weight: 1,
                content_changed: false,
            };
            view.process_delta(&add_delta, &db);
            
            // Delete
            let del_delta = Delta {
                table: SmolStr::new("user"),
                key: SmolStr::new("user:1"),
                weight: -1,
                content_changed: false,
            };
            view.process_delta(&del_delta, &db);
        }
        
        // Should end up empty
        assert!(view.cache.is_empty() || !view.cache.is_member("user:1"));
    }

    /// TEST: Concurrent-style batch (add and delete same record)
    #[test]
    fn test_batch_add_and_delete_same_record() {
        let db = setup_db_with_users(&[("1", "Alice")]);
        let plan = simple_scan_plan("user");
        let mut view = View::new(plan, None, Some(ViewResultFormat::Streaming));
        
        // Batch with +1 and -1 for same record (net 0)
        let mut batch = BatchDeltas::new();
        let mut zset = ZSet::default();
        zset.insert(SmolStr::new("user:1"), 1);
        zset.insert(SmolStr::new("user:1"), -1); // This would set it to 0
        batch.membership.insert("user".to_string(), zset);
        
        // Note: HashMap will only keep one entry, so this tests the final state
        let result = view.process_batch(&batch, &db);
        
        // Net 0 change - should have no events or record
        // Exact behavior depends on order of operations
    }
}

// ============================================================================
// INTEGRATION TEST: YOUR EXACT SCENARIO
// ============================================================================

#[cfg(test)]
mod integration_tests {
    use super::*;

    /// TEST: Your exact scenario from the requirements
    /// 
    /// view1: SELECT * FROM user WHERE id = $id LIMIT 1;
    /// view2: SELECT * FROM user LIMIT 5;
    /// 
    /// 1. user:1 signs up -> 2 edges created
    /// 2. user:2 signs up -> 1 edge created (view2 only)
    /// 3. user:1 updates -> 2 edges updated
    /// 4. user:2 deletes -> 1 edge deleted
    /// 5. view2 unregistered -> edges for view2 deleted
    #[test]
    fn test_your_exact_scenario() {
        let mut db = Database::new();
        db.ensure_table("user");
        
        // View 1: SELECT * FROM user WHERE id = 'user:1' LIMIT 1
        let plan1 = filter_by_id_plan("user", "user:1");
        let mut view1 = View::new(plan1, None, Some(ViewResultFormat::Streaming));
        
        // View 2: SELECT * FROM user LIMIT 5
        let plan2 = limit_plan("user", 5);
        let mut view2 = View::new(plan2, None, Some(ViewResultFormat::Streaming));
        
        // === STEP 1: user:1 signs up ===
        let user_table = db.tables.get_mut("user").unwrap();
        user_table.rows.insert(SmolStr::new("1"), make_user("1", "User 1"));
        user_table.zset.insert(SmolStr::new("user:1"), 1);
        
        let delta1 = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:1"),
            weight: 1,
            content_changed: false,
        };
        
        let result1_v1 = view1.process_delta(&delta1, &db);
        let result1_v2 = view2.process_delta(&delta1, &db);
        
        // Both views should have Created event
        assert!(result1_v1.is_some(), "view1 should emit for user:1");
        assert!(result1_v2.is_some(), "view2 should emit for user:1");
        
        let (created1, _, _) = count_events(&result1_v1.unwrap());
        let (created2, _, _) = count_events(&result1_v2.unwrap());
        assert_eq!(created1, 1, "view1: 1 edge created");
        assert_eq!(created2, 1, "view2: 1 edge created");
        
        // === STEP 2: user:2 signs up ===
        let user_table = db.tables.get_mut("user").unwrap();
        user_table.rows.insert(SmolStr::new("2"), make_user("2", "User 2"));
        user_table.zset.insert(SmolStr::new("user:2"), 1);
        
        let delta2 = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:2"),
            weight: 1,
            content_changed: false,
        };
        
        let result2_v1 = view1.process_delta(&delta2, &db);
        let result2_v2 = view2.process_delta(&delta2, &db);
        
        // view1 filters to user:1, so user:2 should not appear
        assert!(
            result2_v1.is_none() || {
                let (c, _, _) = count_events(&result2_v1.unwrap());
                c == 0
            },
            "view1 should NOT emit for user:2 (filtered out)"
        );
        
        // view2 should see user:2
        assert!(result2_v2.is_some(), "view2 should emit for user:2");
        let (created, _, _) = count_events(&result2_v2.unwrap());
        assert_eq!(created, 1, "view2: 1 edge created for user:2");
        
        // === STEP 3: user:1 updates ===
        let user_table = db.tables.get_mut("user").unwrap();
        user_table.rows.insert(SmolStr::new("1"), make_user("1", "User 1 Updated"));
        
        let delta3 = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:1"),
            weight: 0,
            content_changed: true,
        };
        
        let result3_v1 = view1.process_delta(&delta3, &db);
        let result3_v2 = view2.process_delta(&delta3, &db);
        
        // Both should emit Updated
        assert!(result3_v1.is_some());
        assert!(result3_v2.is_some());
        
        let (_, updated1, _) = count_events(&result3_v1.unwrap());
        let (_, updated2, _) = count_events(&result3_v2.unwrap());
        assert_eq!(updated1, 1, "view1: 1 edge updated");
        assert_eq!(updated2, 1, "view2: 1 edge updated");
        
        // === STEP 4: user:2 deletes ===
        let user_table = db.tables.get_mut("user").unwrap();
        user_table.rows.remove("2");
        user_table.zset.remove("user:2");
        
        let delta4 = Delta {
            table: SmolStr::new("user"),
            key: SmolStr::new("user:2"),
            weight: -1,
            content_changed: false,
        };
        
        let result4_v1 = view1.process_delta(&delta4, &db);
        let result4_v2 = view2.process_delta(&delta4, &db);
        
        // view1 never had user:2
        assert!(
            result4_v1.is_none(),
            "view1 should not emit for user:2 delete (never had it)"
        );
        
        // view2 should emit Deleted
        assert!(result4_v2.is_some());
        let (_, _, deleted) = count_events(&result4_v2.unwrap());
        assert_eq!(deleted, 1, "view2: 1 edge deleted");
        
        // === STEP 5: view2 unregistered ===
        // When view is unregistered, all edges should be deleted
        // This is handled at the edge management level, not view level
        // But we can verify the current state of the cache
        
        // view1 should have: user:1
        assert!(view1.cache.is_member("user:1"));
        assert!(!view1.cache.is_member("user:2"));
        
        // view2 should have: user:1 (user:2 was deleted)
        assert!(view2.cache.is_member("user:1"));
        assert!(!view2.cache.is_member("user:2"));
        
        println!("✅ All steps passed!");
    }
}