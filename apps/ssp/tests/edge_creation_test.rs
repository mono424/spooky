use surrealdb::engine::local::Mem;
use surrealdb::Surreal;
use surrealdb::types::RecordId;
use ssp::engine::update::{StreamingUpdate, DeltaEvent, DeltaRecord};
use tokio;

// Helper to parse record ID (simplified for test)
fn parse_record_id(id: &str) -> Option<RecordId> {
    RecordId::parse_simple(id).ok()
}

fn format_incantation_id(id: &str) -> String {
    if id.starts_with("_spooky_incantation:") {
        id.to_string()
    } else {
        format!("_spooky_incantation:{}", id)
    }
}

// Logic copied from lib_backup.rs (adapted for test)
async fn update_all_edges(db: &Surreal<surrealdb::engine::local::Db>, updates: &[&StreamingUpdate]) {
    if updates.is_empty() { return; }
    
    for (idx, update) in updates.iter().enumerate() {
        if update.records.is_empty() { continue; }
        
        let incantation_id_str = format_incantation_id(&update.view_id);
        let from_id = parse_record_id(&incantation_id_str).expect("Invalid incantation ID");
        let binding_name = format!("from{}", idx);
        
        let mut update_stmts: Vec<String> = Vec::new();
        for record in &update.records {
             let stmt = match record.event {
                DeltaEvent::Created => {
                    format!(
                        "RELATE ${1}->_spooky_list_ref->(SELECT id FROM ONLY _spooky_version WHERE record_id = {0}) 
                            SET version = (SELECT version FROM ONLY _spooky_version WHERE record_id = {0}).version, 
                                clientId = (SELECT clientId FROM ONLY ${1}).clientId",
                        record.id,
                        binding_name
                    )
                }
                DeltaEvent::Updated => {
                    format!(
                        "UPDATE ${1}->_spooky_list_ref SET version += 1 WHERE out = (SELECT id FROM ONLY _spooky_version WHERE record_id = {0})", 
                        record.id, 
                        binding_name
                    )
                }
                DeltaEvent::Deleted => {
                    format!(
                        "DELETE ${1}->_spooky_list_ref WHERE out = (SELECT id FROM ONLY _spooky_version WHERE record_id = {0})", 
                        record.id, 
                        binding_name
                    )
                }
            };
            update_stmts.push(stmt);
        }
        
        if !update_stmts.is_empty() {
             let full_query = format!(
                 "BEGIN TRANSACTION;\n{};\nCOMMIT TRANSACTION;",
                 update_stmts.join(";\n")
             );
             println!("Query: {}", full_query);
             let mut query = db.query(&full_query);
             query = query.bind((binding_name, from_id));
             match query.await {
                Ok(res) => println!("Update Result: {:?}", res),
                Err(e) => eprintln!("Error executing query: {}", e),
            }
        }
    }
}

#[tokio::test]
async fn test_edge_persistence_logic() {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db.use_ns("test").use_db("test").await.unwrap();

    // 1. Setup Version Records for two items
    let id1 = "table:thing1";
    let id2 = "table:thing2";
    
    db.query(&format!("CREATE _spooky_version:thing1 SET record_id = {}, version = 1;", id1)).await.unwrap();
    db.query(&format!("CREATE _spooky_version:thing2 SET record_id = {}, version = 1;", id2)).await.unwrap();

    let view_id = "view_persistent";
    let incantation_id = format_incantation_id(view_id);
    db.query(&format!("CREATE {} SET clientId = 'test_client';", incantation_id)).await.unwrap();

    // 2. First Update: Create Edge 1
    let update1 = StreamingUpdate {
        view_id: view_id.to_string(),
        records: vec![
            DeltaRecord { id: id1.into(), event: DeltaEvent::Created }
        ],
    };
    update_all_edges(&db, &[&update1]).await;

    // Verify Edge 1 exists
    let sql1 = format!("SELECT * FROM _spooky_list_ref WHERE in = {}", incantation_id);
    let mut res1 = db.query(&sql1).await.unwrap();
    let rows1: Vec<serde_json::Value> = res1.take(0).ok().unwrap();
    println!("Rows 1: {:?}", rows1);
    assert_eq!(rows1.len(), 1, "Edge 1 should exist after first update");

    // 3. Second Update: Create Edge 2
    let update2 = StreamingUpdate {
        view_id: view_id.to_string(),
        records: vec![
            DeltaRecord { id: id2.into(), event: DeltaEvent::Created }
        ],
    };
    update_all_edges(&db, &[&update2]).await;

    // 4. Verify Total Edges
    let sql2 = format!("SELECT * FROM _spooky_list_ref WHERE in = {}", incantation_id);
    let mut res2 = db.query(&sql2).await.unwrap();
    let rows2: Vec<serde_json::Value> = res2.take(0).ok().unwrap();
    println!("Rows 2: {:?}", rows2);
    assert_eq!(rows2.len(), 2, "Both edges should exist after second update. If 1, previous edges were deleted!");
}
