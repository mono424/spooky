mod common;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use surrealdb::types::{RecordId, Table};

// Strict Schema without tricks
const SCHEMA_QUERY: &str = "
    DEFINE TABLE user SCHEMAFULL;
    DEFINE FIELD username ON TABLE user TYPE string;

    DEFINE TABLE thread SCHEMAFULL;
    DEFINE FIELD title ON TABLE thread TYPE string;
    // STRICT RECORD TYPE
    DEFINE FIELD author ON TABLE thread TYPE record<user>;
";

#[derive(Debug, Serialize, Deserialize)]
struct User {
    id: Option<RecordId>,
    username: String,
}

// Helper to parse 'table:key' -> RecordId
fn parse_record_id(id_str: &str) -> RecordId {
    let parts: Vec<&str> = id_str.splitn(2, ':').collect();
    if parts.len() != 2 {
        panic!("Invalid RecordId string");
    }
    RecordId {
        table: Table::from(parts[0]),
        key: parts[1].into(),
    }
}

#[tokio::test]
async fn test_client_side_parsing() -> anyhow::Result<()> {
    // 1. Connect
    let db = common::setup_mem().await;
    db.use_db("test".to_string(), "test".to_string()).await?;

    // 2. Load Strict Schema
    db.query(SCHEMA_QUERY.to_string(), None).await?;

    // 3. Create User
    let user_creation = db.query("CREATE user SET username = 'John'".to_string(), None).await?;
    let created_user_val: Value = serde_json::from_str(&user_creation)?;
    let user_id_str = created_user_val[0][0]["id"].as_str().expect("User ID string");
    println!("Created User ID String: {}", user_id_str);

    // 4. PARSE - Client Side
    let author_rid = parse_record_id(user_id_str);
    
    // 5. Create Thread using the RecordId OBJECT in variables
    let vars = json!({
        "data": {
            "title": "Parsed ID Thread",
            "author": author_rid 
        }
    });
    
    // USE TYPED QUERY HERE
    let thread_res = db.query_typed(
        "CREATE thread CONTENT $data".to_string(), 
        Some(vars.to_string())
    ).await?;

    // Check for errors
    let thread_val: Value = serde_json::from_str(&thread_res)?;
    if let Some(err) = thread_val[0].get("error") {
        panic!("Thread creation failed: {:?}", err);
    }
    
    println!("Success! Client-side parsed RecordId was accepted.");
    Ok(())
}
