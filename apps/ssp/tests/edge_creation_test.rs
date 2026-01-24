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
    let mut all_statements: Vec<String> = Vec::new();
    let mut bindings: Vec<(String, RecordId)> = Vec::new();

    for (idx, update) in updates.iter().enumerate() {
        if update.records.is_empty() { continue; }
        let incantation_id_str = format_incantation_id(&update.view_id);
        let from_id = parse_record_id(&incantation_id_str).expect("Invalid incantation ID");
        let binding_name = format!("from{}", idx);
        bindings.push((binding_name.clone(), from_id));

        for record in &update.records {
             let stmt = match record.event {
                DeltaEvent::Created => {
                    format!(
                        "
                        LET $spooky_version = (SELECT id, version FROM ONLY _spooky_version WHERE record_id = {0});
                        RELATE ${1}->_spooky_list_ref->{0} SET version = ($spooky_version.version);
                        ",
                        record.id,
                        binding_name
                    )
                }
                DeltaEvent::Updated => {
                    format!("UPDATE ${}->_spooky_list_ref WHERE out = {}", binding_name, record.id)
                }
                DeltaEvent::Deleted => {
                    format!("DELETE ${}->_spooky_list_ref WHERE out = {}", binding_name, record.id)
                }
            };
            all_statements.push(stmt);
        }
    }

    if all_statements.is_empty() { return; }
    let full_query = format!("BEGIN TRANSACTION;\n{};\nCOMMIT TRANSACTION;", all_statements.join(";\n"));
    println!("Query: {}", full_query);
    let mut query = db.query(&full_query);
    for (name, id) in bindings { query = query.bind((name, id)); }
    if let Err(e) = query.await { eprintln!("Error executing query: {}", e); }
}

#[tokio::test]
async fn test_edge_creation_logic() {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db.use_ns("test").use_db("test").await.unwrap();

    let record_id = "table:thing1";
    let setup_sql = format!(
        "CREATE _spooky_version:thing1 SET record_id = {}, version = 1;", 
        record_id
    );
    db.query(&setup_sql).await.unwrap();

    let view_id = "view_123";
    let update = StreamingUpdate {
        view_id: view_id.to_string(),
        records: vec![
            DeltaRecord {
                id: record_id.into(),
                event: DeltaEvent::Created,
            }
        ],
    };

    update_all_edges(&db, &[&update]).await;

    let incantation_id = format_incantation_id(view_id);
    let sql = format!(
        "SELECT count() AS total FROM _spooky_list_ref WHERE in = {} AND out = {}", 
        incantation_id, record_id
    );
    
    let mut result = db.query(&sql).await.unwrap();
    
    // Debug print result to confirm output
    // Result should look like [ { total: 1 } ]
    println!("VERIFICATION RESULT: {:?}", result);

    result.check().expect("Verification query failed");
}
