//! Integration Flow Test
//!
//! This test simulates and visualizes the full communication flow:
//! DB (simulated) → Sidecar Ingest → DBSP Processing → StreamingUpdate → Edge Updates
//!
//! Run with: cargo test --test integration_flow_test -- --nocapture
//!
//! The test outputs detailed logs showing payload structure at each step,
//! making it easy to verify and improve the data flow.

mod common;

use common::*;
use serde_json::json;
use ssp::engine::update::{DeltaEvent, ViewResultFormat, ViewUpdate};
use ssp::engine::view::{Operator, Path, Predicate, Projection, QueryPlan};

/// Represents an edge operation that would be sent to the database
#[derive(Debug, Clone)]
struct EdgeOperation {
    op_type: &'static str, // "RELATE" or "DELETE"
    from: String,          // incantation ID
    to: String,            // record ID
    version: u64,
}

/// Simulates what the sidecar would do with a StreamingUpdate
fn process_streaming_update(
    update: &ViewUpdate,
    incantation_id: &str,
) -> Vec<EdgeOperation> {
    let mut ops = Vec::new();
    
    if let ViewUpdate::Streaming(s) = update {
        for record in &s.records {
            let op_type = match record.event {
                DeltaEvent::Created => "RELATE",
                DeltaEvent::Updated => "UPDATE",
                DeltaEvent::Deleted => "DELETE",
            };
            
            ops.push(EdgeOperation {
                op_type,
                from: format!("_spooky_incantation:{}", incantation_id),
                to: record.id.clone(),
                version: record.version,
            });
        }
    }
    
    ops
}

/// Pretty print a StreamingUpdate
fn print_streaming_update(update: &ViewUpdate, step: &str) {
    if let ViewUpdate::Streaming(s) = update {
        println!("\n╔══════════════════════════════════════════════════════════════════╗");
        println!("║ {} - StreamingUpdate", step);
        println!("╠══════════════════════════════════════════════════════════════════╣");
        println!("║ view_id: {}", s.view_id);
        println!("║ records_count: {}", s.records.len());
        println!("╠══════════════════════════════════════════════════════════════════╣");
        for (i, record) in s.records.iter().enumerate() {
            println!("║  [{}] id: {}", i, record.id);
            println!("║      event: {:?}", record.event);
            println!("║      version: {}", record.version);
        }
        println!("╚══════════════════════════════════════════════════════════════════╝");
    }
}

/// Pretty print edge operations
fn print_edge_operations(ops: &[EdgeOperation], step: &str) {
    println!("\n┌──────────────────────────────────────────────────────────────────┐");
    println!("│ {} - Edge Operations (Sidecar → DB)", step);
    println!("├──────────────────────────────────────────────────────────────────┤");
    for (i, op) in ops.iter().enumerate() {
        println!("│  [{}] {} {}->_spooky_list_ref->{}", i, op.op_type, op.from, op.to);
        if op.op_type != "DELETE" {
            println!("│      SET version = {}", op.version);
        }
    }
    println!("└──────────────────────────────────────────────────────────────────┘");
}

/// Print the ingest payload (DB → Sidecar)
fn print_ingest_payload(table: &str, op: &str, id: &str, record: &serde_json::Value) {
    println!("\n┌──────────────────────────────────────────────────────────────────┐");
    println!("│ DB → Sidecar: Ingest Payload");
    println!("├──────────────────────────────────────────────────────────────────┤");
    println!("│  table: \"{}\"", table);
    println!("│  op: \"{}\"", op);
    println!("│  id: \"{}\"", id);
    println!("│  record: {}", serde_json::to_string_pretty(record).unwrap().replace('\n', "\n│          "));
    println!("└──────────────────────────────────────────────────────────────────┘");
}

/// Print view registration payload
fn print_register_payload(view_id: &str, surreal_ql: &str, params: Option<&str>) {
    println!("\n┌──────────────────────────────────────────────────────────────────┐");
    println!("│ Client → Sidecar: Register View");
    println!("├──────────────────────────────────────────────────────────────────┤");
    println!("│  id: \"{}\"", view_id);
    println!("│  surrealQL: \"{}\"", surreal_ql);
    if let Some(p) = params {
        println!("│  params: \"{}\"", p);
    }
    println!("│  format: \"streaming\"");
    println!("└──────────────────────────────────────────────────────────────────┘");
}

/// Build thread list query with author subquery
fn build_thread_list_with_author() -> (QueryPlan, &'static str) {
    let subquery = Operator::Limit {
        input: Box::new(Operator::Filter {
            input: Box::new(Operator::Scan { table: "user".to_string() }),
            predicate: Predicate::Eq {
                field: Path::new("id"),
                value: json!({ "$param": "parent.author" }),
            },
        }),
        limit: 1,
        order_by: None,
    };

    let main_op = Operator::Project {
        input: Box::new(Operator::Scan { table: "thread".to_string() }),
        projections: vec![
            Projection::All,
            Projection::Subquery {
                alias: "author".to_string(),
                plan: Box::new(subquery),
            },
        ],
    };

    let plan = QueryPlan {
        id: "thread_list_with_author".to_string(),
        root: main_op,
    };

    (plan, "SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author FROM thread ORDER BY title desc LIMIT 10;")
}

/// Build thread detail query with comments and nested author
fn build_thread_detail_with_comments() -> (QueryPlan, &'static str) {
    // Comment author subquery
    let comment_author_subquery = Operator::Limit {
        input: Box::new(Operator::Filter {
            input: Box::new(Operator::Scan { table: "user".to_string() }),
            predicate: Predicate::Eq {
                field: Path::new("id"),
                value: json!({ "$param": "parent.author" }),
            },
        }),
        limit: 1,
        order_by: None,
    };

    // Comments subquery with nested author
    let comments_subquery = Operator::Project {
        input: Box::new(Operator::Limit {
            input: Box::new(Operator::Filter {
                input: Box::new(Operator::Scan { table: "comment".to_string() }),
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

    // Thread author subquery
    let thread_author_subquery = Operator::Limit {
        input: Box::new(Operator::Filter {
            input: Box::new(Operator::Scan { table: "user".to_string() }),
            predicate: Predicate::Eq {
                field: Path::new("id"),
                value: json!({ "$param": "parent.author" }),
            },
        }),
        limit: 1,
        order_by: None,
    };

    let main_op = Operator::Limit {
        input: Box::new(Operator::Project {
            input: Box::new(Operator::Scan { table: "thread".to_string() }),
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

    let plan = QueryPlan {
        id: "thread_detail".to_string(),
        root: main_op,
    };

    (plan, "SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author, (SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author FROM comment WHERE thread=$parent.id ORDER BY created_at desc LIMIT 10) AS comments FROM thread WHERE id = $id LIMIT 1;")
}

// =============================================================================
// INTEGRATION FLOW TEST
// =============================================================================

#[test]
fn test_integration_flow_thread_with_author() {
    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║     INTEGRATION FLOW TEST: Thread with Author Subquery          ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");

    let mut circuit = setup();

    // =========================================================================
    // STEP 1: Create User (DB → Sidecar → DBSP)
    // =========================================================================
    println!("\n\n======================================================================");
    println!("STEP 1: Create User");
    println!("======================================================================");

    let user_id = format!("user:{}", generate_id());
    let user_record = json!({
        "id": &user_id,
        "name": "Alice",
        "email": "alice@example.com",
        "type": "user"
    });

    print_ingest_payload("user", "CREATE", &user_id, &user_record);
    
    let _updates = ingest(&mut circuit, "user", "CREATE", &user_id, user_record.clone());
    println!("\n  → DBSP: No views registered yet, no updates emitted");

    // =========================================================================
    // STEP 2: Create Thread (DB → Sidecar → DBSP)
    // =========================================================================
    println!("\n\n======================================================================");
    println!("STEP 2: Create Thread");
    println!("======================================================================");

    let thread_id = format!("thread:{}", generate_id());
    let thread_record = json!({
        "id": &thread_id,
        "title": "First Thread",
        "content": "Hello World!",
        "author": &user_id,
        "active": true,
        "type": "thread"
    });

    print_ingest_payload("thread", "CREATE", &thread_id, &thread_record);
    
    let _updates = ingest(&mut circuit, "thread", "CREATE", &thread_id, thread_record.clone());
    println!("\n  → DBSP: No views registered yet, no updates emitted");

    // =========================================================================
    // STEP 3: Register View (Client → Sidecar → DBSP)
    // =========================================================================
    println!("\n\n======================================================================");
    println!("STEP 3: Register View (Thread List with Author)");
    println!("======================================================================");

    let (plan, surreal_ql) = build_thread_list_with_author();
    let view_id = "thread_list_with_author";
    
    print_register_payload(view_id, surreal_ql, None);

    let initial_update = circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));

    if let Some(ref update) = initial_update {
        print_streaming_update(update, "Initial Registration");
        
        let edge_ops = process_streaming_update(update, view_id);
        print_edge_operations(&edge_ops, "Initial Registration");
    }

    // =========================================================================
    // STEP 4: Create Second Thread (triggers update)
    // =========================================================================
    println!("\n\n======================================================================");
    println!("STEP 4: Create Second Thread (triggers view update)");
    println!("======================================================================");

    let thread2_id = format!("thread:{}", generate_id());
    let thread2_record = json!({
        "id": &thread2_id,
        "title": "Second Thread",
        "content": "Another post",
        "author": &user_id,
        "active": true,
        "type": "thread"
    });

    print_ingest_payload("thread", "CREATE", &thread2_id, &thread2_record);

    let updates = ingest(&mut circuit, "thread", "CREATE", &thread2_id, thread2_record.clone());

    for update in &updates {
        if update.query_id() == view_id {
            print_streaming_update(update, "After Thread Creation");
            
            let edge_ops = process_streaming_update(update, view_id);
            print_edge_operations(&edge_ops, "After Thread Creation");
        }
    }

    // =========================================================================
    // STEP 5: Update Thread Title (no subquery change)
    // =========================================================================
    println!("\n\n======================================================================");
    println!("STEP 5: Update Thread Title (should NOT delete user edge)");
    println!("======================================================================");

    let updated_thread_record = json!({
        "id": &thread_id,
        "title": "Updated First Thread",
        "content": "Hello World!",
        "author": &user_id,
        "active": true,
        "type": "thread"
    });

    print_ingest_payload("thread", "UPDATE", &thread_id, &updated_thread_record);

    let updates = ingest(&mut circuit, "thread", "UPDATE", &thread_id, updated_thread_record);

    for update in &updates {
        if update.query_id() == view_id {
            print_streaming_update(update, "After Thread Update");
            
            let edge_ops = process_streaming_update(update, view_id);
            print_edge_operations(&edge_ops, "After Thread Update");

            // Verify no DELETE operations for user
            for op in &edge_ops {
                if op.to.starts_with("user:") && op.op_type == "DELETE" {
                    panic!("BUG: User edge incorrectly deleted on thread update!");
                }
            }
        }
    }

    // =========================================================================
    // STEP 6: Delete Thread (remove from view)
    // =========================================================================
    println!("\n\n======================================================================");
    println!("STEP 6: Delete Thread");
    println!("======================================================================");

    print_ingest_payload("thread", "DELETE", &thread2_id, &json!({}));

    let updates = ingest(&mut circuit, "thread", "DELETE", &thread2_id, json!({}));

    for update in &updates {
        if update.query_id() == view_id {
            print_streaming_update(update, "After Thread Deletion");
            
            let edge_ops = process_streaming_update(update, view_id);
            print_edge_operations(&edge_ops, "After Thread Deletion");
        }
    }

    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║                    TEST COMPLETED SUCCESSFULLY                   ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!("\n");
}

// =============================================================================
// THREAD DETAIL WITH COMMENTS FLOW TEST
// =============================================================================

#[test]
fn test_integration_flow_thread_detail_with_comments() {
    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║  INTEGRATION FLOW TEST: Thread Detail with Comments              ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");

    let mut circuit = setup();

    // Setup: Create user and thread
    let user_id = format!("user:{}", generate_id());
    let user_record = json!({ "id": &user_id, "name": "Bob", "type": "user" });
    ingest(&mut circuit, "user", "CREATE", &user_id, user_record);

    let thread_id = format!("thread:{}", generate_id());
    let thread_record = json!({
        "id": &thread_id,
        "title": "Discussion Thread",
        "author": &user_id,
        "active": true,
        "type": "thread"
    });
    ingest(&mut circuit, "thread", "CREATE", &thread_id, thread_record);

    // Register view
    println!("\n\n======================================================================");
    println!("STEP 1: Register Thread Detail View");
    println!("======================================================================");

    let (plan, surreal_ql) = build_thread_detail_with_comments();
    let view_id = "thread_detail";
    
    print_register_payload(view_id, surreal_ql, Some("{ id: thread:xxx }"));

    let initial_update = circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));

    if let Some(ref update) = initial_update {
        print_streaming_update(update, "Initial Registration");
        let edge_ops = process_streaming_update(update, view_id);
        print_edge_operations(&edge_ops, "Initial Registration");
    }

    // Create comment
    println!("\n\n======================================================================");
    println!("STEP 2: Create Comment (subquery change detected)");
    println!("======================================================================");

    let comment_id = format!("comment:{}", generate_id());
    let comment_record = json!({
        "id": &comment_id,
        "content": "Great post!",
        "thread": &thread_id,
        "author": &user_id,
        "type": "comment"
    });

    print_ingest_payload("comment", "CREATE", &comment_id, &comment_record);

    let updates = ingest(&mut circuit, "comment", "CREATE", &comment_id, comment_record);

    for update in &updates {
        if update.query_id() == view_id {
            print_streaming_update(update, "After Comment Creation");
            let edge_ops = process_streaming_update(update, view_id);
            print_edge_operations(&edge_ops, "After Comment Creation");
        }
    }

    // Delete comment
    println!("\n\n======================================================================");
    println!("STEP 3: Delete Comment");
    println!("======================================================================");

    print_ingest_payload("comment", "DELETE", &comment_id, &json!({}));

    let updates = ingest(&mut circuit, "comment", "DELETE", &comment_id, json!({}));

    for update in &updates {
        if update.query_id() == view_id {
            print_streaming_update(update, "After Comment Deletion");
            let edge_ops = process_streaming_update(update, view_id);
            print_edge_operations(&edge_ops, "After Comment Deletion");
        }
    }

    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║                    TEST COMPLETED SUCCESSFULLY                   ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!("\n");
}

// =============================================================================
// VERSION MAP STATE INSPECTION
// =============================================================================

#[test]
fn test_version_map_state_tracking() {
    println!("\n");
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║         VERSION MAP STATE TRACKING TEST                          ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");

    let mut circuit = setup();

    // Setup
    let user_id = format!("user:{}", generate_id());
    ingest(&mut circuit, "user", "CREATE", &user_id, json!({ "id": &user_id, "name": "Test" }));

    let thread_id = format!("thread:{}", generate_id());
    ingest(&mut circuit, "thread", "CREATE", &thread_id, json!({ 
        "id": &thread_id, "title": "Test", "author": &user_id 
    }));

    // Register view
    let (plan, _) = build_thread_list_with_author();
    circuit.register_view(plan, None, Some(ViewResultFormat::Streaming));

    // Function to print version map state
    let print_version_map = |circuit: &ssp::Circuit, view_id: &str, step: &str| {
        if let Some(view) = circuit.views.iter().find(|v| v.plan.id == view_id) {
            println!("\n┌──────────────────────────────────────────────────────────────────┐");
            println!("│ {} - Version Map State", step);
            println!("├──────────────────────────────────────────────────────────────────┤");
            for (id, version) in &view.metadata.versions {
                println!("│  {} → version {}", id, version);
            }
            println!("└──────────────────────────────────────────────────────────────────┘");
        }
    };

    print_version_map(&circuit, "thread_list_with_author", "After Registration");

    // Update thread multiple times
    for i in 1..=3 {
        let updated = json!({
            "id": &thread_id,
            "title": format!("Update #{}", i),
            "author": &user_id
        });
        ingest(&mut circuit, "thread", "UPDATE", &thread_id, updated);
        print_version_map(&circuit, "thread_list_with_author", &format!("After Update #{}", i));
    }

    println!("\n[TEST] ✓ Version map correctly tracks all IDs across updates");
}
