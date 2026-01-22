//! Regression tests for subquery edge handling in streaming mode.
//!
//! These tests cover bugs found in the SSP streaming pipeline where:
//! 1. Comment deletion incorrectly affected subquery tracking
//! 2. Thread title updates incorrectly removed user edges
//!
//! The key issue was that version_map contains BOTH main query IDs (threads)
//! AND subquery IDs (users, comments), but target_set only contains main query results.
//! This caused subquery IDs to be incorrectly marked as removals.

use common::ViewUpdateExt;
mod common;

use common::*;
use serde_json::json;
use ssp::engine::update::{DeltaEvent, ViewResultFormat, ViewUpdate};
use ssp::{Operator, Path, Predicate, Projection, QueryPlan};

/// Helper to create a user record
fn make_user_record(name: &str) -> (String, serde_json::Value) {
    let id = generate_id();
    let full_id = format!("user:{}", id);
    let record = json!({
        "id": full_id,
        "name": name,
        "type": "user"
    });
    (full_id, record)
}

/// Helper to create a thread record with active flag
fn make_thread_record_with_author(title: &str, author_id: &str) -> (String, serde_json::Value) {
    let id = generate_id();
    let full_id = format!("thread:{}", id);
    let record = json!({
        "id": full_id,
        "title": title,
        "author": author_id,
        "active": true,
        "type": "thread"
    });
    (full_id, record)
}

/// Helper to create a comment record
fn make_comment_for_thread(content: &str, thread_id: &str, author_id: &str) -> (String, serde_json::Value) {
    let id = generate_id();
    let full_id = format!("comment:{}", id);
    let record = json!({
        "id": full_id,
        "content": content,
        "thread": thread_id,
        "author": author_id,
        "type": "comment"
    });
    (full_id, record)
}

/// Build a query plan that mimics:
/// SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author
/// FROM thread ORDER BY title desc LIMIT 10
fn build_thread_list_with_author_plan(plan_id: &str) -> QueryPlan {
    // Subquery: SELECT * FROM user WHERE id = $parent.author LIMIT 1
    let author_subquery = Operator::Limit {
        input: Box::new(Operator::Filter {
            input: Box::new(Operator::Scan {
                table: "user".to_string(),
            }),
            predicate: Predicate::Eq {
                field: Path::new("id"),
                value: json!({ "$param": "parent.author" }),
            },
        }),
        limit: 1,
        order_by: None,
    };

    // Main query: SELECT *, author_subquery FROM thread
    let main_op = Operator::Project {
        input: Box::new(Operator::Scan {
            table: "thread".to_string(),
        }),
        projections: vec![
            Projection::All,
            Projection::Subquery {
                alias: "author".to_string(),
                plan: Box::new(author_subquery),
            },
        ],
    };

    QueryPlan {
        id: plan_id.to_string(),
        root: main_op,
    }
}

/// Build a query plan that mimics:
/// SELECT *, 
///   (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author,
///   (SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author 
///    FROM comment WHERE thread=$parent.id ORDER BY created_at desc LIMIT 10) AS comments
/// FROM thread WHERE id = $id LIMIT 1
fn build_thread_detail_with_comments_plan(plan_id: &str) -> QueryPlan {
    // Author subquery for comment: SELECT * FROM user WHERE id = $parent.author LIMIT 1
    let comment_author_subquery = Operator::Limit {
        input: Box::new(Operator::Filter {
            input: Box::new(Operator::Scan {
                table: "user".to_string(),
            }),
            predicate: Predicate::Eq {
                field: Path::new("id"),
                value: json!({ "$param": "parent.author" }),
            },
        }),
        limit: 1,
        order_by: None,
    };

    // Comments subquery with nested author: SELECT *, author FROM comment WHERE thread = $parent.id
    let comments_subquery = Operator::Project {
        input: Box::new(Operator::Limit {
            input: Box::new(Operator::Filter {
                input: Box::new(Operator::Scan {
                    table: "comment".to_string(),
                }),
                predicate: Predicate::Eq {
                    field: Path::new("thread"),
                    value: json!({ "$param": "parent.id" }),
                },
            }),
            limit: 10,
            order_by: None,
        }),
        projections: vec![
            Projection::All,
            Projection::Subquery {
                alias: "author".to_string(),
                plan: Box::new(comment_author_subquery),
            },
        ],
    };

    // Thread author subquery: SELECT * FROM user WHERE id = $parent.author LIMIT 1
    let thread_author_subquery = Operator::Limit {
        input: Box::new(Operator::Filter {
            input: Box::new(Operator::Scan {
                table: "user".to_string(),
            }),
            predicate: Predicate::Eq {
                field: Path::new("id"),
                value: json!({ "$param": "parent.author" }),
            },
        }),
        limit: 1,
        order_by: None,
    };

    // Main query: SELECT *, author, comments FROM thread WHERE id = $id LIMIT 1
    let main_op = Operator::Limit {
        input: Box::new(Operator::Project {
            input: Box::new(Operator::Scan {
                table: "thread".to_string(),
            }),
            projections: vec![
                Projection::All,
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
    };

    QueryPlan {
        id: plan_id.to_string(),
        root: main_op,
    }
}

/// Extract streaming records from a view update
fn get_streaming_records(update: &ViewUpdate) -> Vec<(&str, &DeltaEvent)> {
    match update {
        ViewUpdate::Streaming(s) => s.records.iter().map(|r| (r.id.as_str(), &r.event)).collect(),
        _ => vec![],
    }
}

/// Check that a specific ID has a specific event type in the streaming update
fn has_event(update: &ViewUpdate, id: &str, expected_event: &DeltaEvent) -> bool {
    if let ViewUpdate::Streaming(s) = update {
        s.records.iter().any(|r| r.id == id && &r.event == expected_event)
    } else {
        false
    }
}

/// Get all IDs in version_map for a specific view
fn get_version_map_ids(circuit: &ssp::Circuit, view_id: &str) -> Vec<String> {
    if let Some(view) = circuit.views.iter().find(|v| v.plan.id == view_id) {
        view.cache.keys().map(|k: &smol_str::SmolStr| k.to_string()).collect()
    } else {
        vec![]
    }
}

// ============================================================================
// TEST 1: Thread title update should NOT delete user edges
// ============================================================================
// This reproduces the bug where updating a thread title incorrectly
// removed user IDs from version_map because they weren't in target_set.

#[test]
fn test_thread_update_preserves_user_edges() {
    let mut circuit = setup();

    // 1. Create user
    let (user_id, user_record) = make_user_record("Alice");
    ingest(&mut circuit, "user", "CREATE", &user_id, user_record);

    // 2. Create thread authored by user
    let (thread_id, thread_record) = make_thread_record_with_author("First Thread", &user_id);
    ingest(&mut circuit, "thread", "CREATE", &thread_id, thread_record);

    // 3. Register streaming view: thread list with author subquery
    let plan = build_thread_list_with_author_plan("thread_list_with_author");
    let initial_update = circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));

    // 4. Verify initial state has both thread and user
    assert!(initial_update.is_some(), "Expected initial update");
    let init_update = initial_update.unwrap();
    let init_records = get_streaming_records(&init_update);
    
    println!("Initial records: {:?}", init_records);
    assert!(
        init_records.iter().any(|(id, event)| *id == thread_id && matches!(event, DeltaEvent::Created)),
        "Expected thread in initial update"
    );
    assert!(
        init_records.iter().any(|(id, event)| *id == user_id && matches!(event, DeltaEvent::Created)),
        "Expected user in initial update (from subquery)"
    );

    // 5. Verify version_map contains both IDs (prefixed with table source)
    let version_ids = get_version_map_ids(&circuit, "thread_list_with_author");
    println!("Version map IDs after register: {:?}", version_ids);
    assert!(version_ids.contains(&format!("thread:{}", thread_id)), "Version map should contain thread");
    assert!(version_ids.contains(&format!("user:{}", user_id)), "Version map should contain user");

    // 6. Update thread title (NOT deleting anything!)
    let updated_thread_record = json!({
        "id": thread_id,
        "title": "Updated Thread Title",
        "author": user_id,
        "active": true,
        "type": "thread"
    });
    let updates = ingest(&mut circuit, "thread", "UPDATE", &thread_id, updated_thread_record);

    // 7. CRITICAL: Verify user is NOT deleted from version_map
    let version_ids_after = get_version_map_ids(&circuit, "thread_list_with_author");
    println!("Version map IDs after update: {:?}", version_ids_after);
    
    assert!(
        version_ids_after.contains(&format!("thread:{}", thread_id)),
        "Thread should still be in version_map after update"
    );
    assert!(
        version_ids_after.contains(&format!("user:{}", user_id)),
        "BUG DETECTED: User was removed from version_map when only thread was updated!"
    );

    // 8. Verify no Deleted events were emitted for user
    for update in &updates {
        if let ViewUpdate::Streaming(s) = update {
            if s.view_id == "thread_list_with_author" {
                for record in &s.records {
                    if record.id == user_id {
                        assert!(
                            !matches!(record.event, DeltaEvent::Deleted),
                            "BUG DETECTED: User should NOT have Deleted event when thread is updated"
                        );
                    }
                }
            }
        }
    }

    println!("[TEST] ✓ Thread update correctly preserves user edges");
}

// ============================================================================
// TEST 2: Comment deletion should be tracked correctly
// ============================================================================
// This reproduces the original bug where deleting a comment wasn't
// properly handled in streaming mode with subqueries.

#[test]
fn test_comment_deletion_streaming() {
    let mut circuit = setup();

    // 1. Create user
    let (user_id, user_record) = make_user_record("Bob");
    ingest(&mut circuit, "user", "CREATE", &user_id, user_record);

    // 2. Create thread
    let (thread_id, thread_record) = make_thread_record_with_author("Thread with Comments", &user_id);
    ingest(&mut circuit, "thread", "CREATE", &thread_id, thread_record);

    // 3. Create first comment
    let (comment1_id, comment1_record) = make_comment_for_thread("First comment", &thread_id, &user_id);
    ingest(&mut circuit, "comment", "CREATE", &comment1_id, comment1_record);

    // 4. Register streaming view: thread detail with comments
    let plan = build_thread_detail_with_comments_plan("thread_detail");
    let initial_update = circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));

    assert!(initial_update.is_some(), "Expected initial update");
    let init_update = initial_update.unwrap();
    let init_records = get_streaming_records(&init_update);
    
    println!("Initial records: {:?}", init_records);
    
    let version_ids = get_version_map_ids(&circuit, "thread_detail");
    println!("Version map after register: {:?}", version_ids);
    assert!(version_ids.contains(&format!("thread:{}", thread_id)), "Should have thread");
    assert!(version_ids.contains(&format!("user:{}", user_id)), "Should have user");
    assert!(version_ids.contains(&format!("comment:{}", comment1_id)), "Should have comment");

    // 6. Create second comment
    let (comment2_id, comment2_record) = make_comment_for_thread("Second comment", &thread_id, &user_id);
    let create_updates = ingest(&mut circuit, "comment", "CREATE", &comment2_id, comment2_record);
    
    // Verify comment2 was added
    for update in &create_updates {
        if update.query_id() == "thread_detail" {
            assert!(
                has_event(update, &comment2_id, &DeltaEvent::Created),
                "Second comment should have Created event"
            );
        }
    }

    let version_ids_after_create = get_version_map_ids(&circuit, "thread_detail");
    assert!(version_ids_after_create.contains(&format!("comment:{}", comment2_id)), "Should have second comment");

    // 7. Delete first comment
    let delete_updates = ingest(&mut circuit, "comment", "DELETE", &comment1_id, json!({}));

    // 8. Verify comment1 has Deleted event
    let mut found_delete = false;
    for update in &delete_updates {
        if update.query_id() == "thread_detail" {
            if has_event(update, &comment1_id, &DeltaEvent::Deleted) {
                found_delete = true;
            }
        }
    }
    assert!(found_delete, "First comment should have Deleted event");

    // 9. Verify comment1 is removed from version_map
    let version_ids_after_delete = get_version_map_ids(&circuit, "thread_detail");
    println!("Version map after delete: {:?}", version_ids_after_delete);
    
    assert!(
        !version_ids_after_delete.contains(&format!("comment:{}", comment1_id)),
        "Deleted comment should NOT be in version_map"
    );
    assert!(
        version_ids_after_delete.contains(&format!("comment:{}", comment2_id)),
        "Second comment should still be in version_map"
    );
    assert!(
        version_ids_after_delete.contains(&format!("user:{}", user_id)),
        "User should still be in version_map"
    );
    assert!(
        version_ids_after_delete.contains(&format!("thread:{}", thread_id)),
        "Thread should still be in version_map"
    );

    println!("[TEST] ✓ Comment deletion correctly tracked in streaming mode");
}

// ============================================================================
// TEST 3: Complex scenario - multiple operations
// ============================================================================
// This reproduces the full user scenario:
// 1. Create user -> Create thread -> Create comment
// 2. Update thread title
// 3. Delete comment
// All edges should be correctly maintained throughout.

#[test]
fn test_full_scenario_edge_tracking() {
    let mut circuit = setup();

    // 1. Create user
    let (user_id, user_record) = make_user_record("Charlie");
    ingest(&mut circuit, "user", "CREATE", &user_id, user_record);

    // 2. Create thread
    let (thread_id, thread_record) = make_thread_record_with_author("Original Title", &user_id);
    ingest(&mut circuit, "thread", "CREATE", &thread_id, thread_record);

    // 3. Register streaming views
    let plan1 = build_thread_list_with_author_plan("v_thread_list");
    circuit.register_view(plan1, None, Some(ViewResultFormat::Streaming));

    let plan2 = build_thread_detail_with_comments_plan("v_thread_detail");
    circuit.register_view(plan2, None, Some(ViewResultFormat::Streaming));

    // 4. Create comment
    let (comment_id, comment_record) = make_comment_for_thread("Test comment", &thread_id, &user_id);
    ingest(&mut circuit, "comment", "CREATE", &comment_id, comment_record);

    // Verify state after comment creation
    let list_ids = get_version_map_ids(&circuit, "v_thread_list");
    let detail_ids = get_version_map_ids(&circuit, "v_thread_detail");
    
    println!("After comment creation:");
    println!("  v_thread_list: {:?}", list_ids);
    println!("  v_thread_detail: {:?}", detail_ids);

    assert!(list_ids.contains(&format!("thread:{}", thread_id)), "List should have thread");
    assert!(list_ids.contains(&format!("user:{}", user_id)), "List should have user");
    assert!(detail_ids.contains(&format!("thread:{}", thread_id)), "Detail should have thread");
    assert!(detail_ids.contains(&format!("user:{}", user_id)), "Detail should have user");
    assert!(detail_ids.contains(&format!("comment:{}", comment_id)), "Detail should have comment");

    // 5. Update thread title
    let updated_thread = json!({
        "id": thread_id,
        "title": "Updated Title",
        "author": user_id,
        "active": true,
        "type": "thread"
    });
    ingest(&mut circuit, "thread", "UPDATE", &thread_id, updated_thread);

    // Verify state after thread update - NO edges should be lost!
    let list_ids_after_update = get_version_map_ids(&circuit, "v_thread_list");
    let detail_ids_after_update = get_version_map_ids(&circuit, "v_thread_detail");
    
    println!("After thread update:");
    println!("  v_thread_list: {:?}", list_ids_after_update);
    println!("  v_thread_detail: {:?}", detail_ids_after_update);

    assert!(list_ids_after_update.contains(&format!("thread:{}", thread_id)), "List should still have thread");
    assert!(
        list_ids_after_update.contains(&format!("user:{}", user_id)),
        "BUG: List lost user after thread update!"
    );
    assert!(detail_ids_after_update.contains(&format!("thread:{}", thread_id)), "Detail should still have thread");
    assert!(
        detail_ids_after_update.contains(&format!("user:{}", user_id)),
        "BUG: Detail lost user after thread update!"
    );
    assert!(
        detail_ids_after_update.contains(&format!("comment:{}", comment_id)),
        "Detail should still have comment"
    );

    // 6. Delete comment
    ingest(&mut circuit, "comment", "DELETE", &comment_id, json!({}));

    // Verify state after comment deletion
    let list_ids_after_delete = get_version_map_ids(&circuit, "v_thread_list");
    let detail_ids_after_delete = get_version_map_ids(&circuit, "v_thread_detail");
    
    println!("After comment deletion:");
    println!("  v_thread_list: {:?}", list_ids_after_delete);
    println!("  v_thread_detail: {:?}", detail_ids_after_delete);

    assert!(list_ids_after_delete.contains(&format!("thread:{}", thread_id)), "List should still have thread");
    assert!(list_ids_after_delete.contains(&format!("user:{}", user_id)), "List should still have user");
    assert!(detail_ids_after_delete.contains(&format!("thread:{}", thread_id)), "Detail should still have thread");
    assert!(detail_ids_after_delete.contains(&format!("user:{}", user_id)), "Detail should still have user");
    assert!(
        !detail_ids_after_delete.contains(&format!("comment:{}", comment_id)),
        "Detail should NOT have deleted comment"
    );

    println!("[TEST] ✓ Full scenario edge tracking works correctly");
}

// ============================================================================
// TEST 4: Ensure subquery table IDs are never incorrectly deleted
// ============================================================================
// This directly tests the fix we applied: subquery table IDs should
// never be marked as removals when only main table records change.

#[test]
fn test_subquery_ids_not_marked_as_removals_on_main_table_update() {
    let mut circuit = setup();

    // Setup: user + thread
    let (user_id, user_record) = make_user_record("Diana");
    ingest(&mut circuit, "user", "CREATE", &user_id, user_record);

    let (thread_id, thread_record) = make_thread_record_with_author("Test Thread", &user_id);
    ingest(&mut circuit, "thread", "CREATE", &thread_id, thread_record);

    // Register view
    let plan = build_thread_list_with_author_plan("subquery_test");
    circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));

    // Record initial state
    let initial_ids = get_version_map_ids(&circuit, "subquery_test");
    let initial_user_present = initial_ids.contains(&format!("user:{}", user_id));
    let initial_thread_present = initial_ids.contains(&format!("thread:{}", thread_id));

    println!("Initial version_map: {:?}", initial_ids);
    assert!(initial_user_present, "User should be tracked initially");
    assert!(initial_thread_present, "Thread should be tracked initially");

    // Perform multiple thread updates
    for i in 1..=5 {
        let updated = json!({
            "id": thread_id,
            "title": format!("Update #{}", i),
            "author": user_id,
            "active": true,
            "type": "thread"
        });
        let updates = ingest(&mut circuit, "thread", "UPDATE", &thread_id, updated);

        // After EACH update, verify user is still present
        let ids = get_version_map_ids(&circuit, "subquery_test");
        
        assert!(
            ids.contains(&format!("user:{}", user_id)),
            "BUG at update #{}: User was incorrectly removed from version_map", i
        );
        assert!(
            ids.contains(&format!("thread:{}", thread_id)),
            "Thread should still be in version_map after update #{}", i
        );

        // Also verify no Deleted events for user
        for update in &updates {
            if let ViewUpdate::Streaming(s) = update {
                if s.view_id == "subquery_test" {
                    for record in &s.records {
                        if record.id == user_id {
                            assert!(
                                !matches!(record.event, DeltaEvent::Deleted),
                                "BUG at update #{}: User incorrectly got Deleted event", i
                            );
                        }
                    }
                }
            }
        }
    }

    println!("[TEST] ✓ Subquery IDs correctly preserved across {} updates", 5);
}
