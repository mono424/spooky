//! End-to-End Communication Test
//!
//! This test simulates the full communication flow:
//! DB (simulated) → Sidecar → Circuit.ingest_record → ViewUpdate → DB persistence (simulated)
//!
//! The test validates:
//! 1. Payload structure for Flat and Streaming formats
//! 2. Simulated DB operations (RELATE, UPDATE, DELETE for _spooky_list_ref)
//! 3. Complete round-trip data integrity

mod common;

use common::*;
use serde_json::json;
use ssp::engine::update::{DeltaEvent, ViewResultFormat, ViewUpdate};
use ssp::{Operator, Path, Predicate, Projection, QueryPlan};
use smol_str::SmolStr;

/// Simulated DB operation for testing persistence logic
#[derive(Debug, Clone, PartialEq)]
enum DbOperation {
    /// UPDATE incantations SET hash = $hash, array = $array WHERE id = $incantation_id
    UpdateIncantation {
        incantation_id: String,
        hash: String,
        array: Vec<SmolStr>,
    },
    /// RELATE $from->_spooky_list_ref->$to
    RelateEdge {
        from: String,
        to: SmolStr,
        // version removed
    },
    /// UPDATE $from->_spooky_list_ref SET ... WHERE out = $to
    UpdateEdge {
        from: String,
        to: SmolStr,
        // version removed
    },
    /// DELETE $from->_spooky_list_ref WHERE out = $to
    DeleteEdge {
        from: String,
        to: SmolStr,
    },
}

/// Simulates the sidecar's update_incantation_in_db() function
fn simulate_update_flat(view_id: &str, update: &ViewUpdate) -> Option<DbOperation> {
    match update {
        ViewUpdate::Flat(m) | ViewUpdate::Tree(m) => {
            assert_eq!(m.query_id, view_id, "View ID mismatch");
            Some(DbOperation::UpdateIncantation {
                incantation_id: format!("_spooky_query:{}", view_id),
                hash: m.result_hash.clone(),
                array: m.result_data.clone(),
            })
        }
        _ => None,
    }
}

/// Simulates the sidecar's update_incantation_edges() function
fn simulate_update_streaming(view_id: &str, update: &ViewUpdate) -> Vec<DbOperation> {
    let mut ops = Vec::new();
    
    if let ViewUpdate::Streaming(s) = update {
        assert_eq!(s.view_id, view_id, "View ID mismatch");
        
        let from = format!("_spooky_query:{}", view_id);
        
        for record in &s.records {
            let op = match record.event {
                DeltaEvent::Created => DbOperation::RelateEdge {
                    from: from.clone(),
                    to: record.id.clone(),
                },
                DeltaEvent::Updated => DbOperation::UpdateEdge {
                    from: from.clone(),
                    to: record.id.clone(),
                },
                DeltaEvent::Deleted => DbOperation::DeleteEdge {
                    from: from.clone(),
                    to: record.id.clone(),
                },
            };
            ops.push(op);
        }
    }
    
    ops
}

/// Helper to create a user record
fn make_user(name: &str) -> (String, serde_json::Value) {
    let id = generate_id();
    let full_id = format!("user:{}", id);
    let record = json!({
        "id": full_id,
        "name": name,
        "type": "user"
    });
    (full_id, record)
}

/// Helper to create a thread record
fn make_thread(title: &str, author_id: &str) -> (String, serde_json::Value) {
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
fn make_comment(content: &str, thread_id: &str, author_id: &str) -> (String, serde_json::Value) {
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

/// Build query plan: SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author FROM thread
fn build_thread_with_author_plan(plan_id: &str) -> QueryPlan {
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

// ============================================================================
// TEST 1: Flat Format - Complete Payload Flow
// ============================================================================

#[test]
fn test_flat_format_e2e_flow() {
    println!("\n=== TEST: Flat Format E2E Flow ===\n");
    
    let mut circuit = setup();
    let mut db_ops: Vec<DbOperation> = Vec::new();

    // Phase 1: Create data
    println!("PHASE 1: CREATE");
    let (user_id, user_record) = make_user("Alice");
    ingest(&mut circuit, "user", "CREATE", &user_id, user_record);
    
    let (thread_id, thread_record) = make_thread("First Thread", &user_id);
    ingest(&mut circuit, "thread", "CREATE", &thread_id, thread_record);

    // Phase 2: Register view in Flat format
    println!("PHASE 2: REGISTER VIEW (Flat)");
    let plan = build_thread_with_author_plan("thread_flat");
    let reg_update = circuit.register_view(plan, None, Some(ViewResultFormat::Flat));
    
    assert!(reg_update.is_some(), "Registration should return update");
    let update = reg_update.unwrap();
    
    // Simulate sidecar persistence
    if let Some(op) = simulate_update_flat("thread_flat", &update) {
        println!("  → DB OP: {:?}", op);
        db_ops.push(op.clone());
        
        // Validate payload
        if let DbOperation::UpdateIncantation { hash, array, .. } = op {
            assert!(!hash.is_empty(), "Hash should not be empty");
            assert_eq!(array.len(), 2, "Should have thread + user");
            
            // Verify both records are present
            let ids: Vec<String> = array.iter().map(|s| s.to_string()).collect();
            assert!(ids.contains(&thread_id), "Array should contain thread");
            assert!(ids.contains(&user_id), "Array should contain user (from subquery)");
            println!("  ✓ Flat payload validated: {} records, hash={}", array.len(), &hash[..8]);
        }
    }

    // Phase 3: Update thread
    println!("PHASE 3: UPDATE");
    let updated_thread = json!({
        "id": thread_id,
        "title": "Updated Title",
        "author": user_id,
        "active": true,
        "type": "thread"
    });
    let updates = ingest(&mut circuit, "thread", "UPDATE", &thread_id, updated_thread);
    
    for update in &updates {
        if let Some(op) = simulate_update_flat("thread_flat", update) {
            println!("  → DB OP: {:?}", op);
            db_ops.push(op);
        }
    }

    // Phase 4: Delete thread
    println!("PHASE 4: DELETE");
    let delete_updates = ingest(&mut circuit, "thread", "DELETE", &thread_id, json!({}));
    
    for update in &delete_updates {
        if let Some(op) = simulate_update_flat("thread_flat", update) {
            println!("  → DB OP: {:?}", op);
            db_ops.push(op.clone());
            
            // After deletion, array should only have user (if still referenced elsewhere) or be empty
            if let DbOperation::UpdateIncantation { array, .. } = op {
                println!("  ✓ After delete: {} records remaining", array.len());
            }
        }
    }

    println!("\n=== SUMMARY ===");
    println!("Total DB operations: {}", db_ops.len());
    println!("All operations:");
    for (i, op) in db_ops.iter().enumerate() {
        println!("  [{}] {:?}", i + 1, op);
    }
    
    println!("\n✅ Flat format E2E test passed!\n");
}

// ============================================================================
// TEST 2: Streaming Format - Complete Payload Flow with Graph Operations
// ============================================================================

#[test]
fn test_streaming_format_e2e_flow() {
    println!("\n=== TEST: Streaming Format E2E Flow ===\n");
    
    let mut circuit = setup();
    let mut db_ops: Vec<DbOperation> = Vec::new();

    // Phase 1: Create data
    println!("PHASE 1: CREATE DATA");
    let (user_id, user_record) = make_user("Bob");
    ingest(&mut circuit, "user", "CREATE", &user_id, user_record);
    
    let (thread_id, thread_record) = make_thread("Streaming Thread", &user_id);
    ingest(&mut circuit, "thread", "CREATE", &thread_id, thread_record);

    // Phase 2: Register view in Streaming format
    println!("PHASE 2: REGISTER VIEW (Streaming)");
    let plan = build_thread_with_author_plan("thread_streaming");
    let reg_update = circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));
    
    assert!(reg_update.is_some(), "Registration should return update");
    let update = reg_update.unwrap();
    
    // Simulate sidecar streaming persistence
    let ops = simulate_update_streaming("thread_streaming", &update);
    println!("  → DB OPS: {} graph operations", ops.len());
    
    // Validate initial Created events
    let relate_ops: Vec<_> = ops.iter()
        .filter(|op| matches!(op, DbOperation::RelateEdge { .. }))
        .collect();
    
    assert_eq!(relate_ops.len(), 2, "Should create edges for thread + user");
    
    for op in &ops {
        println!("    {:?}", op);
        db_ops.push(op.clone());
    }
    
    println!("  ✓ Initial edges created: {} RELATE operations", relate_ops.len());

    // Phase 3: Update thread (should trigger Updated event)
    println!("PHASE 3: UPDATE THREAD");
    let updated_thread = json!({
        "id": thread_id,
        "title": "Updated Streaming Title",
        "author": user_id,
        "active": true,
        "type": "thread"
    });
    let updates = ingest(&mut circuit, "thread", "UPDATE", &thread_id, updated_thread);
    
    for update in &updates {
        let ops = simulate_update_streaming("thread_streaming", update);
        
        if !ops.is_empty() {
            println!("  → DB OPS: {} operations", ops.len());
            
            // Validate Updated event for thread (user should NOT be deleted)
            let update_edge_ops: Vec<_> = ops.iter()
                .filter(|op| matches!(op, DbOperation::UpdateEdge { .. }))
                .collect();
            
            let delete_ops: Vec<_> = ops.iter()
                .filter(|op| matches!(op, DbOperation::DeleteEdge { .. }))
                .collect();
            
            println!("    UPDATE operations: {}", update_edge_ops.len());
            println!("    DELETE operations: {}", delete_ops.len());
            
            // CRITICAL: User should NOT have a delete operation
            for op in &delete_ops {
                if let DbOperation::DeleteEdge { to, .. } = op {
                    assert_ne!(to.as_str(), user_id.as_str(), "BUG: User should not be deleted when thread is updated!");
                }
            }
            
            for op in &ops {
                println!("    {:?}", op);
                db_ops.push(op.clone());
            }
        }
    }

    // Phase 4: Create comment (should trigger Created event)
    println!("PHASE 4: CREATE COMMENT");
    let (comment_id, comment_record) = make_comment("Test comment", &thread_id, &user_id);
    let comment_updates = ingest(&mut circuit, "comment", "CREATE", &comment_id, comment_record);
    
    for update in &comment_updates {
        let ops = simulate_update_streaming("thread_streaming", update);
        if !ops.is_empty() {
            println!("  → DB OPS: {} operations", ops.len());
            for op in &ops {
                println!("    {:?}", op);
                db_ops.push(op.clone());
            }
        }
    }

    // Phase 5: Delete comment (should trigger Deleted event)
    println!("PHASE 5: DELETE COMMENT");
    let delete_updates = ingest(&mut circuit, "comment", "DELETE", &comment_id, json!({}));
    
    for update in &delete_updates {
        let ops = simulate_update_streaming("thread_streaming", update);
        
        if !ops.is_empty() {
            println!("  → DB OPS: {} operations", ops.len());
            
            // Validate Deleted event
            let delete_ops: Vec<_> = ops.iter()
                .filter(|op| matches!(op, DbOperation::DeleteEdge { .. }))
                .collect();
            
            assert!(!delete_ops.is_empty(), "Should have DELETE operations for removed comment");
            
            for op in &ops {
                println!("    {:?}", op);
                db_ops.push(op.clone());
            }
            
            println!("  ✓ Comment deletion: {} DELETE operations", delete_ops.len());
        }
    }

    // Phase 6: Final validation - delete thread
    println!("PHASE 6: DELETE THREAD");
    let final_deletes = ingest(&mut circuit, "thread", "DELETE", &thread_id, json!({}));
    
    for update in &final_deletes {
        let ops = simulate_update_streaming("thread_streaming", update);
        if !ops.is_empty() {
            println!("  → DB OPS: {} operations", ops.len());
            for op in &ops {
                println!("    {:?}", op);
                db_ops.push(op.clone());
            }
        }
    }

    println!("\n=== SUMMARY ===");
    println!("Total DB operations: {}", db_ops.len());
    
    let total_relate = db_ops.iter().filter(|op| matches!(op, DbOperation::RelateEdge { .. })).count();
    let total_update = db_ops.iter().filter(|op| matches!(op, DbOperation::UpdateEdge { .. })).count();
    let total_delete = db_ops.iter().filter(|op| matches!(op, DbOperation::DeleteEdge { .. })).count();
    
    println!("  RELATE operations: {}", total_relate);
    println!("  UPDATE operations: {}", total_update);
    println!("  DELETE operations: {}", total_delete);
    
    println!("\nAll operations:");
    for (i, op) in db_ops.iter().enumerate() {
        println!("  [{}] {:?}", i + 1, op);
    }
    
    println!("\n✅ Streaming format E2E test passed!\n");
}

// ============================================================================
// TEST 3: Payload Comparison - Flat vs Streaming
// ============================================================================

#[test]
fn test_payload_comparison_flat_vs_streaming() {
    println!("\n=== TEST: Payload Comparison (Flat vs Streaming) ===\n");
    
    let mut circuit_flat = setup();
    let mut circuit_streaming = setup();

    // Setup identical data
    let (user_id, user_record) = make_user("Charlie");
    let (thread_id, thread_record) = make_thread("Comparison Thread", &user_id);
    
    ingest(&mut circuit_flat, "user", "CREATE", &user_id, user_record.clone());
    ingest(&mut circuit_flat, "thread", "CREATE", &thread_id, thread_record.clone());
    
    ingest(&mut circuit_streaming, "user", "CREATE", &user_id, user_record);
    ingest(&mut circuit_streaming, "thread", "CREATE", &thread_id, thread_record);

    // Register views with different formats
    let plan_flat = build_thread_with_author_plan("view_flat");
    let plan_streaming = build_thread_with_author_plan("view_streaming");
    
    let flat_update = circuit_flat.register_view(plan_flat, None, Some(ViewResultFormat::Flat));
    let streaming_update = circuit_streaming.register_view(plan_streaming, None, Some(ViewResultFormat::Streaming));

    // Compare payloads
    println!("FLAT PAYLOAD:");
    if let Some(ViewUpdate::Flat(m)) = &flat_update {
        println!("  query_id: {}", m.query_id);
        println!("  result_hash: {}", &m.result_hash[..16]);
        println!("  result_data ({} records):", m.result_data.len());
        for id in &m.result_data {
            println!("    - {}", id);
        }
    }

    println!("\nSTREAMING PAYLOAD:");
    if let Some(ViewUpdate::Streaming(s)) = &streaming_update {
        println!("  view_id: {}", s.view_id);
        println!("  records ({} delta events):", s.records.len());
        for rec in &s.records {
            println!("    - {:?}: {}", rec.event, rec.id);
        }
    }

    // Validate consistency
    if let (Some(ViewUpdate::Flat(m)), Some(ViewUpdate::Streaming(s))) = (&flat_update, &streaming_update) {
        println!("\nCONSISTENCY CHECK:");
        
        // Both should track the same IDs
        let flat_ids: std::collections::HashSet<_> = m.result_data.iter().collect();
        let streaming_ids: std::collections::HashSet<_> = s.records.iter().map(|r| &r.id).collect();
        
        assert_eq!(flat_ids, streaming_ids, "Flat and Streaming should track the same record IDs");
        println!("  ✓ Both formats track {} records", flat_ids.len());
        println!("  ✓ ID sets match perfectly");
    }
    
    println!("\n✅ Payload comparison test passed!\n");
}
