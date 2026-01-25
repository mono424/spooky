use common::ViewUpdateExt;
mod common;

use common::*;
use serde_json::json;
use ssp::{Operator, QueryPlan};

/// Test that registering a view AFTER records are ingested correctly finds existing records
#[test]
fn test_view_registration_after_ingestion() {
    let mut circuit = setup();

    // 1. Ingest a user record BEFORE registering the view
    let user_id = format!("user:{}", generate_id());
    let user_record = json!({
        "id": user_id,
        "username": "testuser",
    });

    println!(
        "[TEST] Ingesting user before view registration: {}",
        user_id
    );
    ingest(
        &mut circuit,
        "user",
        "CREATE",
        &user_id,
        user_record.clone(),
    );

    // 2. Verify the record is in the database
    assert!(
        circuit.db.tables.contains_key("user"),
        "User table should exist"
    );
    let user_table = &circuit.db.tables["user"];
    assert!(
        user_table
            .zset
            .contains_key(user_id.as_str()),
        "User should be in zset"
    );
    assert!(
        user_table.rows.contains_key(user_id.as_str()),
        "User should be in rows"
    );
    println!("[TEST] User found in database zset and rows");

    // 3. NOW register a view that queries the user table
    let plan = QueryPlan {
        id: "user_query".to_string(),
        root: Operator::Scan {
            table: "user".to_string(),
        },
    };

    println!("[TEST] Registering view after ingestion");
    let initial_update = circuit.register_view(plan, None, None);

    // 4. The initial update should contain the user that was already in the database
    assert!(
        initial_update.is_some(),
        "Initial update should not be None"
    );

    let update = initial_update.unwrap();
    println!(
        "[TEST] Initial update result_data: {:?}",
        update.result_data()
    );

    assert_eq!(update.result_data().len(), 1, "Should find 1 user");
    assert_eq!(
        update.result_data()[0], user_id,
        "Should find the correct user"
    );
    // assert!(update.result_data()[0].1 > 0, "Version should be positive"); // Version not in result data anymore

    println!("[TEST] ✓ View correctly found pre-existing record!");
}

/// Test with more complex query plan (with filter)
#[test]
fn test_view_registration_after_ingestion_with_filter() {
    let mut circuit = setup();

    // 1. Ingest multiple users BEFORE registering the view
    let user1_id = format!("user:{}", generate_id());
    let user1_record = json!({
        "id": user1_id,
        "username": "alice",
        "active": true,
    });

    let user2_id = format!("user:{}", generate_id());
    let user2_record = json!({
        "id": user2_id,
        "username": "bob",
        "active": false,
    });

    println!("[TEST] Ingesting users before view registration");
    ingest(
        &mut circuit,
        "user",
        "CREATE",
        &user1_id,
        user1_record.clone(),
    );
    ingest(
        &mut circuit,
        "user",
        "CREATE",
        &user2_id,
        user2_record.clone(),
    );

    // 2. Register a view that filters for active users only
    use ssp::{Path, Predicate};

    let plan = QueryPlan {
        id: "active_users".to_string(),
        root: Operator::Filter {
            input: Box::new(Operator::Scan {
                table: "user".to_string(),
            }),
            predicate: Predicate::Eq {
                field: Path::new("active"),
                value: json!(true),
            },
        },
    };

    println!("[TEST] Registering filtered view after ingestion");
    let initial_update = circuit.register_view(plan, None, None);

    // 3. Should only find the active user
    assert!(
        initial_update.is_some(),
        "Initial update should not be None"
    );

    let update = initial_update.unwrap();
    println!(
        "[TEST] Filtered update result_data: {:?}",
        update.result_data()
    );

    assert_eq!(update.result_data().len(), 1, "Should find 1 active user");
    assert_eq!(
        update.result_data()[0], user1_id,
        "Should find alice (active user)"
    );

    println!("[TEST] ✓ Filtered view correctly found pre-existing active record!");
}
