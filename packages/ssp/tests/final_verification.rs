use ssp::{Circuit, engine::{QueryPlan, Operator, update::ViewResultFormat, metadata::IngestStrategy}};

fn create_simple_plan(id: &str) -> QueryPlan {
    QueryPlan {
        id: id.to_string(),
        root: Operator::Scan {
            table: "test_table".to_string(),
        },
    }
}

#[test]
fn test_backward_compat_register_view() {
    let mut circuit = Circuit::new();
    let plan = create_simple_plan("view1");
    // Should compile with 3 arguments (backward compatibility)
    let _ = circuit.register_view(plan, None, Some(ViewResultFormat::Flat));
    assert!(true);
}

#[test]
fn test_register_view_with_strategy() {
    let mut circuit = Circuit::new();
    let plan = create_simple_plan("view2");
    // Should compile with 4 arguments
    let _ = circuit.register_view_with_strategy(
        plan, 
        None, 
        Some(ViewResultFormat::Flat),
        Some(IngestStrategy::Explicit)
    );
    assert!(true);
}

#[test]
fn test_build_materialized_performance() {
    let mut circuit = Circuit::new();
    // Register materialized view
    let plan = create_simple_plan("perf_view");
    circuit.register_view(plan, None, Some(ViewResultFormat::Flat));

    // Ingest MANY records to fill the view
    let num_records = 10_000;
    let mut batch = Vec::new();
    for i in 0..num_records {
        batch.push((
            "test_table".to_string(),
            "CREATE".to_string(),
            format!("id:{}", i),
            serde_json::json!({"id": format!("id:{}", i), "val": i}),
            format!("hash:{}", i),
        ));
    }
    circuit.ingest_batch_outdated(batch);

    // Now perform a batch update that modifies MANY records
    // This triggers `build_materialized_raw_result` which had the O(n^2) bug
    let mut update_batch = Vec::new();
    for i in 0..num_records {
        if i % 2 == 0 { // Update half of them
            update_batch.push((
                "test_table".to_string(),
                "UPDATE".to_string(),
                format!("id:{}", i),
                serde_json::json!({"id": format!("id:{}", i), "val": i + 1000}),
                format!("hash_new:{}", i),
            ));
        }
    }

    let start = std::time::Instant::now();
    circuit.ingest_batch_outdated(update_batch);
    let duration = start.elapsed();

    println!("Update of {} records took {:?}", num_records / 2, duration);
    
    // With O(n^2), 5000 updates on 10000 records:
    // 5000 * 10000 iterations = 50,000,000 checks.
    // In Rust this might be fast, but if it was O(n^2) it would be noticeably slower than O(n).
    // O(n) would be ~15000 checks (hash lookups).
    // If it takes < 100ms it's likely fixed. (50M checks might take > 100ms in debug).
    
    // Assert roughly fast execution (generous limit for debug build)
    assert!(duration.as_millis() < 500, "Performance O(n^2) check failed, took too long: {:?}", duration);
}
