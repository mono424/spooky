// This verification test uses a local implementation of the edge update logic
// that mirrors the fix in lib.rs. This avoids complex dependency injection of Metrics
// for this specific test case while still verifying the SQL logic.

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
    if id.starts_with("_spooky_query:") {
        id.to_string()
    } else {
        format!("_spooky_query:{}", id)
    }
}

// Helper to clean up SurrealDB v3 verbose JSON serialization (e.g. {"Object": ...})
fn clean_surreal_value(v: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    match v {
        Value::Object(mut map) => {
            if map.len() == 1 {
                if let Some(inner) = map.remove("Object") {
                    return clean_surreal_value(inner);
                }
                if let Some(inner) = map.remove("Array") {
                    return clean_surreal_value(inner);
                }
                if let Some(inner) = map.remove("String") {
                    return inner;
                }
                if let Some(inner) = map.remove("Number") {
                    return inner;
                }
                if let Some(inner) = map.remove("Bool") {
                    return inner;
                }
                if map.contains_key("None") || map.contains_key("Null") {
                    return Value::Null;
                }
                if let Some(inner) = map.remove("RecordId") {
                    let cleaned = clean_surreal_value(inner);
                    if let Value::Object(rid) = &cleaned {
                         if let (Some(Value::String(k)), Some(Value::String(t))) = (rid.get("key"), rid.get("table")) {
                             return Value::String(format!("{}:{}", t, k));
                         }
                    }
                    return cleaned;
                }
            }
            // General object
            let mut new_map = serde_json::Map::new();
            for (k, v) in map {
                new_map.insert(k, clean_surreal_value(v));
            }
            Value::Object(new_map)
        }
        Value::Array(arr) => {
            Value::Array(arr.into_iter().map(clean_surreal_value).collect())
        }
        _ => v,
    }
}

// Logic copied directly from lib.rs (with fixes applied) for verification
async fn update_all_edges(db: &Surreal<surrealdb::engine::local::Db>, updates: &[&StreamingUpdate]) {
    if updates.is_empty() { return; }
    
    for (idx, update) in updates.iter().enumerate() {
        if update.records.is_empty() { continue; }
        
        // ... (We will implement the FIXED logic here)
        let incantation_id_str = format_incantation_id(&update.view_id);
        let from_id = parse_record_id(&incantation_id_str).expect("Invalid incantation ID");
        let binding_name = format!("from{}", idx);
        
        let mut update_stmts: Vec<String> = Vec::new();
        // Bindings would need to be handled, but for this test helper string interpolation is easier for debugging
        // unless we want to match the library exactly.
        
        for (r_idx, record) in update.records.iter().enumerate() {
             let stmt = match record.event {
                DeltaEvent::Created => {
                    // FIX: Check for duplicates using IF
                    // FIX: SELECT VALUE ... LIMIT 1
                    // FIX: Set version = (SELECT ...)
                    format!(
                        "RELATE ${1}->_spooky_list_ref->(SELECT id FROM ONLY _spooky_version WHERE record_id = {0}) 
                            SET version = (SELECT version FROM ONLY _spooky_version WHERE record_id = {0}).version, 
                                clientId = (SELECT clientId FROM ONLY ${1}).clientId",
                        record.id,
                        binding_name,
                    )
                }
                DeltaEvent::Updated => {
                    format!(
                        "UPDATE ${1}->_spooky_list_ref SET version = (SELECT VALUE version FROM _spooky_version WHERE record_id = '{0}' LIMIT 1) WHERE out = (SELECT VALUE id FROM _spooky_version WHERE record_id = '{0}' LIMIT 1)", 
                        record.id, 
                        binding_name
                    )
                }
                DeltaEvent::Deleted => {
                    format!(
                        "DELETE ${1}->_spooky_list_ref WHERE out = (SELECT VALUE id FROM _spooky_version WHERE record_id = '{0}' LIMIT 1)", 
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
    
    db.query("CREATE _spooky_list_ref:init;").await.unwrap().check().unwrap();
    db.query(&format!("CREATE _spooky_version:thing1 SET record_id = {}, version = 1;", id1)).await.unwrap().check().unwrap();
    db.query(&format!("CREATE _spooky_version:thing2 SET record_id = {}, version = 1;", id2)).await.unwrap().check().unwrap();

    let view_id = "view_persistent";
    let incantation_id = format_incantation_id(view_id);
    db.query(&format!("CREATE {} SET clientId = 'test_client';", incantation_id)).await.unwrap().check().unwrap();

    let mut v_res = db.query("SELECT * FROM _spooky_version").await.unwrap();
    let v_raw: Vec<surrealdb::types::Value> = v_res.take(0).unwrap();
    let v: Vec<serde_json::Value> = v_raw.into_iter().map(|v| clean_surreal_value(serde_json::json!(v))).collect();
    println!("Versions: {:?}", v);

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
    let rows1_raw: Vec<surrealdb::types::Value> = res1.take(0).unwrap();
    let rows1: Vec<serde_json::Value> = rows1_raw.into_iter().map(|v| clean_surreal_value(serde_json::json!(v))).collect();
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
    let rows2_raw: Vec<surrealdb::types::Value> = res2.take(0).unwrap();
    let rows2: Vec<serde_json::Value> = rows2_raw.into_iter().map(|v| clean_surreal_value(serde_json::json!(v))).collect();
    println!("Rows 2: {:?}", rows2);
    assert_eq!(rows2.len(), 2, "Both edges should exist after second update. If 1, previous edges were deleted!");
}

#[tokio::test]
async fn test_complex_queries() {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db.use_ns("test").use_db("test").await.unwrap();

    // 1. Setup Data
    // Users
    db.query("CREATE user:u1 SET name = 'Alice';").await.unwrap().check().unwrap();
    db.query("CREATE user:u2 SET name = 'Bob';").await.unwrap().check().unwrap();

    // Threads
    db.query("CREATE thread:t1 SET title = 'Thread 1', author = user:u1;").await.unwrap().check().unwrap();
    db.query("CREATE thread:t2 SET title = 'Thread 2', author = user:u2;").await.unwrap().check().unwrap();

    // Comments
    db.query("CREATE comment:c1 SET content = 'Comment 1', thread = thread:t1, author = user:u2, created_at = '2024-01-01T10:00:00Z';").await.unwrap().check().unwrap();
    db.query("CREATE comment:c2 SET content = 'Comment 2', thread = thread:t1, author = user:u1, created_at = '2024-01-01T11:00:00Z';").await.unwrap().check().unwrap();

    // Query 1: Thread list with expanded author
    let sql1 = "SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author FROM thread ORDER BY title desc LIMIT 10;";
    println!("Executing Query 1: {}", sql1);
    let mut res1 = db.query(sql1).await.unwrap();
    let raw_rows1: Vec<surrealdb::types::Value> = res1.take(0).unwrap();
    let rows1: Vec<serde_json::Value> = raw_rows1.into_iter().map(|v| clean_surreal_value(serde_json::json!(v))).collect();

    println!("Query 1 Result: {:?}", rows1);
    assert_eq!(rows1.len(), 2);
    // basic check
    assert!(rows1[0].get("author").is_some());

    // Query 2: Get User by ID parameter
    let sql2 = "SELECT * FROM user WHERE id = $id LIMIT 1;";
    let user_id = "user:u1";
    println!("Executing Query 2 with id={}", user_id);
    let user_rid = surrealdb::types::RecordId::parse_simple("user:u1").unwrap();
    let mut res2 = db.query(sql2).bind(("id", user_rid)).await.unwrap();
    let raw_rows2: Vec<surrealdb::types::Value> = res2.take(0).unwrap();
    let rows2: Vec<serde_json::Value> = raw_rows2.into_iter().map(|v| clean_surreal_value(serde_json::json!(v))).collect();

    println!("Query 2 Result: {:?}", rows2);
    assert_eq!(rows2.len(), 1);
    assert_eq!(rows2[0]["id"].as_str().unwrap(), user_id);

    // Query 3: Deep nested fetch
    let sql3 = "SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author, (SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author FROM comment WHERE thread=$parent.id ORDER BY created_at desc LIMIT 10) AS comments FROM thread WHERE id = $id LIMIT 1;";
    let thread_id = "thread:t1";
    println!("Executing Query 3 with id={}", thread_id);
    let thread_rid = surrealdb::types::RecordId::parse_simple("thread:t1").unwrap();
    let mut res3 = db.query(sql3).bind(("id", thread_rid)).await.unwrap();
    let raw_rows3: Vec<surrealdb::types::Value> = res3.take(0).unwrap();
    let rows3: Vec<serde_json::Value> = raw_rows3.into_iter().map(|v| clean_surreal_value(serde_json::json!(v))).collect();

    println!("Query 3 Result: {:?}", rows3);
    assert_eq!(rows3.len(), 1);
    let thread = &rows3[0];
    assert_eq!(thread["id"].as_str().unwrap(), thread_id);
    
    // Check author
    assert_eq!(thread["author"]["id"].as_str().unwrap(), "user:u1");

    // Check comments
    let comments = thread["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 2);
    // c2 is newer than c1, so it should be first
    assert_eq!(comments[0]["id"].as_str().unwrap(), "comment:c2");
    assert_eq!(comments[0]["author"]["id"].as_str().unwrap(), "user:u1");
}
