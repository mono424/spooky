use ssp::engine::circuit::Circuit;
use ssp::engine::update::{StreamingUpdate, DeltaEvent, ViewUpdate, ViewResultFormat, DeltaRecord};
use surrealdb::engine::local::Mem;
use surrealdb::Surreal;
use std::sync::Arc;
use tokio::sync::RwLock;
use serde_json::json;

#[derive(serde::Deserialize, Debug)]
struct VersionRec {
    id: String // or RecordId, let's try String first as Surreal handles conversion often
}

use surrealdb::types::RecordId;
use surrealdb::Connection;

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

async fn update_all_edges<C: Connection>(db: &Surreal<C>, updates: &[&StreamingUpdate]) {
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
                        LET $spooky_version = (SELECT id, version FROM ONLY _spooky_version WHERE ref_id = '{0}');
                        LET $target = $spooky_version.id;
                        RELATE ${1}->_spooky_list_ref->$target SET version = ($spooky_version.version);
                        ",
                        record.id,
                        binding_name
                    )
                }
                DeltaEvent::Updated => {
                     format!(
                        "
                        LET $spooky_version = (SELECT id, version FROM ONLY _spooky_version WHERE ref_id = '{0}');
                        UPDATE ${1}->_spooky_list_ref SET version += 1 WHERE out = $spooky_version.id;
                        ",
                        record.id,
                        binding_name
                    )
                }
                DeltaEvent::Deleted => {
                     format!(
                        "
                        LET $spooky_version = (SELECT id, version FROM ONLY _spooky_version WHERE ref_id = '{0}');
                        DELETE ${1}->_spooky_list_ref WHERE out = $spooky_version.id;
                        ",
                        record.id,
                        binding_name
                    )
                }
            };
            all_statements.push(stmt);
        }
    }

    if !all_statements.is_empty() {
        let full_query = format!("BEGIN TRANSACTION;\n{};\nCOMMIT TRANSACTION;", all_statements.join(";\n"));
        println!("DEBUG QUERY:\n{}", full_query);
        let mut query = db.query(&full_query);
        for (name, id) in bindings { query = query.bind((name, id)); }
        if let Err(e) = query.await { eprintln!("Error executing update_all_edges: {}", e); }
    }
}

#[tokio::test]
async fn test_full_scenario() {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db.use_ns("test").use_db("test").await.unwrap();
    db.query("DEFINE TABLE _spooky_version SCHEMALESS;").await.unwrap();
    db.query("DEFINE TABLE _spooky_list_ref SCHEMALESS;").await.unwrap();
    
    let circuit = Arc::new(RwLock::new(Circuit::new()));

    // ======================================================
    // 1. Create User
    // ======================================================
    let user_id = "user:alice";
    // Create Version
    let v_sql = format!("INSERT INTO _spooky_version (ref_id, version) VALUES ('{}', 1);", user_id);
    println!("INSERT SQL: {}", v_sql);
    let res: Vec<serde_json::Value> = db.query(&v_sql).await.unwrap().take(0).unwrap();
    println!("INSERT RES: {:?}", res);
    let _updates_1: Vec<ViewUpdate> = {
        let mut p = circuit.write().await;
        let entry = ssp::engine::circuit::dto::BatchEntry::new(
            "user",
            ssp::engine::types::Operation::Create,
            user_id,
            json!({"id": user_id, "name": "Alice"}).into()
        );
        p.ingest_single(entry).to_vec()
    };
    
    
    // ======================================================
    // 2. Register View 1 (User by ID)
    // ======================================================
    let view_id_1 = "view_user";
    let payload_1 = json!({
        "id": view_id_1,
        "surql": "SELECT * FROM user WHERE id = $id LIMIT 1",
        "params": { "id": user_id },
        "clientId": "test",
        "ttl": "1h",
        "lastActiveAt": "2024-01-01"
    });
    
    let prep_1 = ssp::service::view::prepare_registration(payload_1).unwrap();
    let init_update_1 = {
        let mut p = circuit.write().await;
        p.register_view(prep_1.plan, prep_1.safe_params, Some(ViewResultFormat::Streaming))
    };
    println!("Init Update 1: {:?}", init_update_1);
    
    println!("Versions: {:?}", db.query("SELECT * FROM _spooky_version").await.unwrap().take::<Vec<serde_json::Value>>(0).unwrap());
    
    if let Some(ViewUpdate::Streaming(s)) = &init_update_1 {
        update_all_edges(&db, &[&s]).await;
    }
    
    // VERIFY View 1
    {
        let incantation_id = format!("_spooky_query:{}", view_id_1);
        let version_id = format!("_spooky_version:{}", user_id.replace(":", "_"));
        let q = format!("SELECT count() as total FROM _spooky_list_ref WHERE in = {} AND out = {} GROUP ALL", incantation_id, version_id);
        let count_res: Vec<serde_json::Value> = db.query(&q).await.unwrap().take(0).unwrap();
        let count = count_res[0]["total"].as_i64().unwrap();
        assert_eq!(count, 1, "Verify V1->User");
    }
    

    // ======================================================
    // 3. Create Thread
    // ======================================================
    let thread_id = "thread:t1";
    // Create Version
    let v_sql = format!("INSERT INTO _spooky_version (ref_id, version) VALUES ('{}', 1);", thread_id);
    println!("INSERT SQL: {}", v_sql);
    let res: Vec<serde_json::Value> = db.query(&v_sql).await.unwrap().take(0).unwrap();
    println!("INSERT RES: {:?}", res);
    
    let updates_thread: Vec<ViewUpdate> = {
        let mut p = circuit.write().await;
        let entry = ssp::engine::circuit::dto::BatchEntry::new(
            "thread", 
            ssp::engine::types::Operation::Create, 
            thread_id, 
            json!({"id": thread_id, "title": "My Thread", "author": user_id}).into()
        );
        p.ingest_single(entry).to_vec()
    };
    let streaming_updates: Vec<&StreamingUpdate> = updates_thread.iter().filter_map(|u| if let ViewUpdate::Streaming(s) = u { Some(s) } else { None }).collect();
    update_all_edges(&db, &streaming_updates).await;
    

    // ======================================================
    // 4. Register View 2 (Thread List)
    // ======================================================
    let view_id_2 = "view_threads";
    let payload_2 = json!({
        "id": view_id_2,
        "surql": "SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author FROM thread ORDER BY title desc LIMIT 10",
        "params": {},
        "clientId": "test",
        "ttl": "1h",
        "lastActiveAt": "2024-01-01"
    });
    
    let prep_2 = ssp::service::view::prepare_registration(payload_2).unwrap();
    let init_update_2 = {
        let mut p = circuit.write().await;
        p.register_view(prep_2.plan, prep_2.safe_params, Some(ViewResultFormat::Streaming))
    };
    
    if let Some(ViewUpdate::Streaming(s)) = init_update_2 {
        update_all_edges(&db, &[&s]).await;
    }

    // VERIFY View 2
    {
        let incantation_id = format!("_spooky_query:{}", view_id_2);
        // Check T1 (Manual ID construction)
        let vid_t1 = format!("_spooky_version:{}", thread_id.replace(":", "_"));
        let ct1_res: Vec<serde_json::Value> = db.query(&format!("SELECT count() as total FROM _spooky_list_ref WHERE in = {} AND out = {} GROUP ALL", incantation_id, vid_t1)).await.unwrap().take(0).unwrap();
        assert_eq!(ct1_res[0]["total"].as_i64().unwrap(), 1, "Verify V2->Thread");
        
        // Check Alice
        let vid_u1 = format!("_spooky_version:{}", user_id.replace(":", "_"));
        let cu1_res: Vec<serde_json::Value> = db.query(&format!("SELECT count() as total FROM _spooky_list_ref WHERE in = {} AND out = {} GROUP ALL", incantation_id, vid_u1)).await.unwrap().take(0).unwrap();
        assert_eq!(cu1_res[0]["total"].as_i64().unwrap(), 1, "Verify V2->User");
    }


    // ======================================================
    // 5. Register View 3 (Complex / Detail)
    // ======================================================
    let view_id_3 = "view_detail";
    let payload_3 = json!({
        "id": view_id_3,
        "surql": "SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author, (SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author FROM comment WHERE thread=$parent.id ORDER BY created_at desc LIMIT 10) AS comments FROM thread WHERE id = $id LIMIT 1",
        "params": { "id": thread_id },
        "clientId": "test",
        "ttl": "1h",
        "lastActiveAt": "2024-01-01"
    });
    
    let prep_3 = ssp::service::view::prepare_registration(payload_3).unwrap();
    let init_update_3 = {
        let mut p = circuit.write().await;
        p.register_view(prep_3.plan, prep_3.safe_params, Some(ViewResultFormat::Streaming))
    };
    
    if let Some(ViewUpdate::Streaming(s)) = init_update_3 {
        update_all_edges(&db, &[&s]).await;
    }
    
    // VERIFY View 3 exists
    {
         let incantation_id = format!("_spooky_query:{}", view_id_3);
         let edges_res: Vec<serde_json::Value> = db.query(&format!("SELECT count() as total FROM _spooky_list_ref WHERE in = {} GROUP ALL", incantation_id)).await.unwrap().take(0).unwrap();
         assert!(edges_res[0]["total"].as_i64().unwrap() > 0, "V3 Edges exist");
    }


    // ======================================================
    // 6. Create Comment (Triggers Update)
    // ======================================================
    let comment_id = "comment:c1";
    // Create Version
    let v_sql = format!("INSERT INTO _spooky_version (ref_id, version) VALUES ('{}', 1);", comment_id);
    println!("INSERT SQL: {}", v_sql);
    let res: Vec<serde_json::Value> = db.query(&v_sql).await.unwrap().take(0).unwrap();
    println!("INSERT RES: {:?}", res);
    
    let updates_comment: Vec<ssp::engine::update::ViewUpdate> = {
        let mut p = circuit.write().await;
        let entry = ssp::engine::circuit::dto::BatchEntry::new(
            "comment", 
            ssp::engine::types::Operation::Create, 
            comment_id, 
            json!({
                "id": comment_id, 
                "text": "Great post!", 
                "thread": thread_id, 
                "author": user_id, 
                "created_at": "2024-01-02"
            }).into()
        );
        p.ingest_single(entry).to_vec()
    };
    
    let streaming_updates_c: Vec<&StreamingUpdate> = updates_comment.iter().filter_map(|u| if let ViewUpdate::Streaming(s) = u { Some(s) } else { None }).collect();
    println!("Comment Updates: {:?}", streaming_updates_c);
    update_all_edges(&db, &streaming_updates_c).await;
    
    // VERIFY View 3 now has Comment
    {
        let incantation_id = format!("_spooky_query:{}", view_id_3);
        let vid_c1 = format!("_spooky_version:{}", comment_id.replace(":", "_"));
        let cc1_res: Vec<serde_json::Value> = db.query(&format!("SELECT count() as total FROM _spooky_list_ref WHERE in = {} AND out = {} GROUP ALL", incantation_id, vid_c1)).await.unwrap().take(0).unwrap();
        assert_eq!(cc1_res[0]["total"].as_i64().unwrap(), 1, "Verify V3->Comment");
    }
    
    println!("Scenario verified successfully!");
}
