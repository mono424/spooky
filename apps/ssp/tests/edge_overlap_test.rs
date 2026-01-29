use ssp::engine::circuit::Circuit;
use ssp::engine::update::{StreamingUpdate, DeltaEvent, DeltaRecord};
use ssp_server::update_all_edges;
use ssp_server::metrics::Metrics;
use surrealdb::engine::local::Mem;
use surrealdb::Surreal;
use std::sync::Arc;
use tokio::sync::RwLock;

// Test: 3 initial edges + 2 new edges (1 overlapping) = 5 total edges
#[tokio::test]
async fn test_edge_overlap_count() {
    // 1. Setup Memory DB
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db.use_ns("test").use_db("test").await.unwrap();

    // 2. Setup Version Records (Mocking existence of data)
    db.query("CREATE _spooky_version:r1 SET record_id = table:r1, version = 1, id = _spooky_version:r1").await.unwrap();
    db.query("CREATE _spooky_version:r2 SET record_id = table:r2, version = 1, id = _spooky_version:r2").await.unwrap();
    db.query("CREATE _spooky_version:r3 SET record_id = table:r3, version = 1, id = _spooky_version:r3").await.unwrap();
    db.query("CREATE _spooky_version:r4 SET record_id = table:r4, version = 1, id = _spooky_version:r4").await.unwrap();

    // 3. Setup Metrics (required by update_all_edges)
    let (_, metrics_val) = ssp_server::metrics::init_metrics().unwrap();
    let metrics = metrics_val; // Struct, not Arc needed for ref? update_all_edges takes &Metrics

    // 4. Scenario: View 1 returns 3 records
    let update1 = StreamingUpdate {
        view_id: "view_1".to_string(),
        records: vec![
            DeltaRecord { id: "table:r1".into(), event: DeltaEvent::Created },
            DeltaRecord { id: "table:r2".into(), event: DeltaEvent::Created },
            DeltaRecord { id: "table:r3".into(), event: DeltaEvent::Created },
        ]
    };

    // 5. Scenario: View 2 returns 2 records (r3 overlaps with view 1, r4 is new)
    let update2 = StreamingUpdate {
        view_id: "view_2".to_string(),
        records: vec![
            DeltaRecord { id: "table:r3".into(), event: DeltaEvent::Created }, // Overlap
            DeltaRecord { id: "table:r4".into(), event: DeltaEvent::Created }, // New
        ]
    };

    // Execute updates
    update_all_edges(&db, &[&update1, &update2], &metrics).await;

    // 6. Verify Total Edge Count
    // Expected: 
    // View 1 -> 3 edges (r1, r2, r3)
    // View 2 -> 2 edges (r3, r4)
    // Total = 5
    
    let sql = "SELECT count() AS total FROM _spooky_list_ref GROUP ALL";
    let mut res = db.query(sql).await.unwrap();
    let result: Vec<serde_json::Value> = res.take(0).unwrap();
    
    println!("Count Result: {:?}", result);
    
    assert!(!result.is_empty(), "Result should not be empty");
    let count = result[0]["total"].as_i64().unwrap();
    assert_eq!(count, 5, "Total edges should be 5");
    
    // Verify specific edges exist
    // Check View 1 -> r3
    let v1_r3 = db.query("SELECT * FROM _spooky_list_ref WHERE in = _spooky_query:view_1 AND out = _spooky_version:r3").await.unwrap().take::<Vec<serde_json::Value>>(0).unwrap();
    assert_eq!(v1_r3.len(), 1, "View 1 should link to r3");

    // Check View 2 -> r3
    let v2_r3 = db.query("SELECT * FROM _spooky_list_ref WHERE in = _spooky_query:view_2 AND out = _spooky_version:r3").await.unwrap().take::<Vec<serde_json::Value>>(0).unwrap();
    assert_eq!(v2_r3.len(), 1, "View 2 should ALSO link to r3 (Overlap)");
}
