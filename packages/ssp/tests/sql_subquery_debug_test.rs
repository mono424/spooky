use common::ViewUpdateExt;
mod common;

use common::*;
use serde_json::json;
use ssp::converter::convert_surql_to_dbsp;
use ssp::engine::view::Operator;

/// Debug the SQL conversion for the TypeScript test SQL
#[test]
fn test_sql_conversion_for_ts_test() {
    // This is the exact SQL from the TypeScript test
    let sql = "SELECT *, (SELECT * FROM author WHERE id = $parent.author)[0] as author_data FROM thread LIMIT 100";

    let result = convert_surql_to_dbsp(sql);
    assert!(result.is_ok(), "Failed to parse SQL: {:?}", result.err());

    let plan_json = result.unwrap();
    println!("=== Parsed JSON ===");
    println!("{}", serde_json::to_string_pretty(&plan_json).unwrap());

    // Try to deserialize to Operator
    let operator: Result<Operator, _> = serde_json::from_value(plan_json);
    assert!(
        operator.is_ok(),
        "Failed to deserialize: {:?}",
        operator.err()
    );

    let op = operator.unwrap();
    println!("\n=== Operator Structure ===");
    println!("{:#?}", op);

    // Check the structure
    match &op {
        Operator::Limit { input, .. } => {
            match input.as_ref() {
                Operator::Project { projections, .. } => {
                    println!("\n=== Projections ===");
                    for (i, proj) in projections.iter().enumerate() {
                        println!("Projection {}: {:?}", i, proj);
                    }

                    // Check if we have a subquery projection
                    let has_subquery = projections.iter().any(|p| {
                        matches!(
                            p,
                            ssp::engine::view::Projection::Subquery { .. }
                        )
                    });
                    assert!(has_subquery, "Expected a Subquery projection");
                }
                _ => panic!("Expected Project inside Limit, got {:?}", input),
            }
        }
        _ => panic!("Expected Limit at top level, got {:?}", op),
    }
}

/// Now test with the same SQL but through the full Circuit flow
#[test]
fn test_subquery_via_sql_full_flow() {
    let mut circuit = setup();

    // 1. Create author and thread
    let (author_id, author_record) = make_author_record("Alice");
    ingest(&mut circuit, "author", "CREATE", &author_id, author_record, true);

    let (thread_id, thread_record) = make_thread_record("Hello World", &author_id);
    ingest(&mut circuit, "thread", "CREATE", &thread_id, thread_record, true);

    // 2. Register view via SQL (using the service layer like WASM does)
    let sql = "SELECT *, (SELECT * FROM author WHERE id = $parent.author)[0] as author_data FROM thread LIMIT 100";

    // Use the service layer's prepare_registration like WASM does
    let config = json!({
        "id": "test_sql_subquery_view",
        "surrealQL": sql,
        "params": {},
        "clientId": "test-client",
        "ttl": "3600s",
        "lastActiveAt": "2026-01-15T00:00:00Z"
    });

    let data = ssp::service::view::prepare_registration(config);
    assert!(
        data.is_ok(),
        "Failed to prepare registration: {:?}",
        data.err()
    );

    let data = data.unwrap();
    println!("=== Plan ID: {} ===", data.plan.id);
    println!("=== Plan Root ===");
    println!("{:#?}", data.plan.root);

    // 3. Register the view
    let update = circuit.register_view(data.plan, data.safe_params, None);
    assert!(update.is_some(), "Expected view update");

    let view_update = update.unwrap();
    println!("\n=== View Update ===");
    println!("query_id: {}", view_update.query_id());
    println!("result_data: {:?}", view_update.result_data());

    // Flat array includes both thread AND author (from subquery)
    let result_ids: Vec<&str> = view_update
        .result_data()
        .iter()
        .map(|(id, _)| id.as_str())
        .collect();
    assert!(
        result_ids.contains(&thread_id.as_str()),
        "Should contain thread ID"
    );
    assert!(
        result_ids.contains(&author_id.as_str()),
        "Should contain author ID from subquery"
    );
    assert_eq!(
        view_update.result_data().len(),
        2,
        "Should have 2 records (thread + author)"
    );
}

/// Helper to create author record
fn make_author_record(name: &str) -> (String, serde_json::Value) {
    let id = generate_id();
    let full_id = format!("author:{}", id);
    let record = json!({
        "id": full_id,
        "name": name,
        "type": "author"
    });
    (full_id, record)
}

/// Helper to create thread record
fn make_thread_record(title: &str, author_id: &str) -> (String, serde_json::Value) {
    let id = generate_id();
    let full_id = format!("thread:{}", id);
    let record = json!({
        "id": full_id,
        "title": title,
        "author": author_id,
        "type": "thread"
    });
    (full_id, record)
}
