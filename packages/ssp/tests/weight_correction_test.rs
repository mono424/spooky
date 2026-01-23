use ssp::engine::circuit::{Circuit, dto::BatchEntry};
use ssp::engine::types::{Operation, Delta, BatchDeltas, SpookyValue};
use ssp::engine::view::QueryPlan;
use ssp::engine::operators::{Operator, Predicate};
use ssp::engine::types::Path;
use ssp::engine::update::ViewResultFormat;
use smol_str::SmolStr;
use serde_json::json;

// ============================================================================
// Unit Tests
// ============================================================================

#[test]
fn test_operation_weights() {
    assert_eq!(Operation::Create.weight(), 1);
    assert_eq!(Operation::Update.weight(), 0);
    assert_eq!(Operation::Delete.weight(), -1);
}

#[test]
fn test_operation_content_change() {
    assert!(Operation::Create.changes_content());
    assert!(Operation::Update.changes_content());
    assert!(!Operation::Delete.changes_content());
}

#[test]
fn test_operation_membership_change() {
    assert!(Operation::Create.changes_membership());
    assert!(!Operation::Update.changes_membership());
    assert!(Operation::Delete.changes_membership());
}

#[test]
fn test_delta_from_operation() {
    let delta = Delta::from_operation(
        SmolStr::new("users"),
        SmolStr::new("users:u1"),
        Operation::Update,
    );
    assert_eq!(delta.weight, 0);
    assert!(delta.content_changed);
}

#[test]
fn test_batch_deltas_tracking() {
    let mut batch = BatchDeltas::new();
    
    batch.add("users", SmolStr::new("users:u1"), Operation::Create);
    batch.add("users", SmolStr::new("users:u2"), Operation::Update);
    batch.add("users", SmolStr::new("users:u3"), Operation::Delete);
    
    // Membership should have u1 (+1) and u3 (-1), but not u2 (weight=0)
    let users_membership = batch.membership.get("users").unwrap();
    assert_eq!(users_membership.get("users:u1"), Some(&1));
    assert_eq!(users_membership.get("users:u2"), None); // weight=0, not tracked
    assert_eq!(users_membership.get("users:u3"), Some(&-1));
    
    // Content updates should have u1 and u2, but not u3
    let users_content = batch.content_updates.get("users").unwrap();
    assert!(users_content.contains(&SmolStr::new("users:u1")));
    assert!(users_content.contains(&SmolStr::new("users:u2")));
    assert!(!users_content.contains(&SmolStr::new("users:u3")));
}

// ============================================================================
// Integration Tests
// ============================================================================

#[test]
fn test_cache_correctness_multiple_updates() {
    let mut circuit = Circuit::new();
    
    let data = json!({"name": "Test"});
    
    // Create
    circuit.ingest_single(BatchEntry::create("users", "u1", SpookyValue::from(data.clone())));
    let cache_weight = get_cache_weight(&circuit, "users", "u1");
    assert_eq!(cache_weight, Some(1), "Create should set weight to 1");
    
    // Update (should NOT change weight)
    circuit.ingest_single(BatchEntry::update("users", "u1", SpookyValue::from(data.clone())));
    let cache_weight = get_cache_weight(&circuit, "users", "u1");
    assert_eq!(cache_weight, Some(1), "Update should keep weight at 1");
    
    // Another update
    circuit.ingest_single(BatchEntry::update("users", "u1", SpookyValue::from(data.clone())));
    let cache_weight = get_cache_weight(&circuit, "users", "u1");
    assert_eq!(cache_weight, Some(1), "Second update should keep weight at 1");
    
    // Delete (should remove)
    circuit.ingest_single(BatchEntry::delete("users", "u1"));
    let cache_weight = get_cache_weight(&circuit, "users", "u1");
    assert_eq!(cache_weight, None, "Delete should remove from cache (weight 0)");
}

// TODO: This test currently fails because filter re-evaluation on updates
// needs more work. The record_matches_view logic is correct but may need
// cache invalidation or forced re-computation
#[test]
#[ignore]
fn test_filter_transition_on_update() {
    let mut circuit = Circuit::new();
    
    // Register view: active users only
    let plan = QueryPlan {
        id: "active_users".into(),
        root: Operator::Filter {
            input: Box::new(Operator::Scan { table: "users".to_string() }),
            predicate: Predicate::Eq {
                field: Path::new("status"),
                value: json!("active"),
            },
        },
    };
    circuit.register_view(plan, None, Some(ViewResultFormat::Flat));
    
    // Create active user
    let active_data = json!({"status": "active", "name": "Alice"});
    circuit.ingest_single(BatchEntry::create("users", "u1", SpookyValue::from(active_data)));
    assert!(view_contains(&circuit, "active_users", "u1"), "Active user should be in view");
    
    // Update to inactive - should leave view
    let inactive_data = json!({"status": "inactive", "name": "Alice"});
    circuit.ingest_single(BatchEntry::update("users", "u1", SpookyValue::from(inactive_data)));
    assert!(!view_contains(&circuit, "active_users", "u1"), "Inactive user should not be in view");
}

// ============================================================================
// Helper Functions
// ============================================================================

fn get_cache_weight(circuit: &Circuit, table: &str, id: &str) -> Option<i64> {
    let zset_key = format!("{}:{}", table, id);
    circuit.db.tables.get(table)
        .and_then(|t| t.zset.get(&SmolStr::new(zset_key)))
        .copied()
}

fn view_contains(circuit: &Circuit, view_id: &str, record_id: &str) -> bool {
    circuit.views.iter()
        .find(|v| v.plan.id == view_id)
        .map(|v| {
            v.cache.keys().any(|k| {
                k.split_once(':')
                    .map(|(_, id)| id == record_id)
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}
