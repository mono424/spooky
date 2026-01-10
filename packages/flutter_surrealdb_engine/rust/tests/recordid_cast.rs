mod common;
use serde::{Deserialize, Serialize};
use serde_json::{from_str, json, Value};
use surrealdb::types::RecordId;

const SCHEMA_QUERY: &str = "
    DEFINE TABLE user SCHEMAFULL;
    DEFINE FIELD username ON TABLE user TYPE string;
    
    // Generic Schema Solution:
    // Use TYPE any to bypass initial strict check, but VALUE to enforce conversion.
    // type::record($value) will convert 'user:id' string to a Record Link.
    // If $value is None/Null, it returns properly.
    // Use ASSERT later if we want to ensure it is specifically a user record, 
    // but type::record ensures it is a valid record format.
    DEFINE FIELD author ON TABLE thread TYPE any 
        VALUE type::record($value);
";

#[derive(Debug, Serialize, Deserialize)]
struct User {
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<RecordId>,
    username: String,
}

#[tokio::test]
async fn test_record_id_cast_integration() -> anyhow::Result<()> {
    // 1. Connect to DB
    let db = common::setup_mem().await;
    db.use_db("test".to_string(), "test".to_string()).await?;

    // 2. Load Schema
    db.query(SCHEMA_QUERY.to_string(), None).await?;

    // 3. Create User
    let user_content = User {
        id: None,
        username: "John Doe".to_string(),
    };
    let user_json_str = db.query(
        "CREATE user CONTENT $data".to_string(),
        Some(json!({ "data": user_content }).to_string())
    ).await?;
    
    // Deserialize result
    println!("Create User Response: {}", user_json_str);
    let created_user_val: Value = serde_json::from_str(&user_json_str)?;
    let user_obj = &created_user_val[0][0];
    let user_valid_id_str = user_obj["id"].as_str().expect("ID should be string");
    println!("User RID String from JSON: {}", user_valid_id_str);

    // 4. Create Thread
    // CRITICAL FINDING:
    // With strict schema `TYPE record<user>`, passing a JSON string "user:id" FAILS.
    // Implicit coercion (even with VALUE clause) is not reliable in strict mode here.
    // The ONLY way to support generic string input while maintaining Graph Traversal
    // is to explicitly cast in the query: `SET author = type::record($val)`.
    
    let thread_vars = json!({
        "title": "My Cast Thread",
        "author": user_valid_id_str 
    });

    let thread_json_res = db.query(
        "CREATE thread SET title = $title, author = type::record($author)".to_string(),
        Some(json!({ 
            "title": "My Cast Thread", 
            "author": user_valid_id_str 
        }).to_string())
    ).await?;
    
    println!("Create Thread Response: {}", thread_json_res);
    let thread_res_val: Value = serde_json::from_str(&thread_json_res)?;
    
    // Check if successful
    if let Some(err) = thread_res_val[0].get("error") {
        panic!("Thread creation failed: {:?}", err);
    }
    
    let thread_obj = &thread_res_val[0][0];
    let author_res = thread_obj["author"].as_str().expect("Author should be record string");
    assert_eq!(author_res, user_valid_id_str);

    // 5. Verify Graph Traversal
    let traverse_res = db.query("SELECT author.username FROM thread".to_string(), None).await?;
    println!("Traversal Response: {}", traverse_res);
    let traverse_json: Value = serde_json::from_str(&traverse_res)?;
    let first_res = &traverse_json[0][0];
    
    if let Some(author_obj) = first_res.get("author") {
        if let Some(username) = author_obj.get("username") {
             assert_eq!(username.as_str().unwrap(), "John Doe");
             println!("Graph traversal successful!");
        } else {
             panic!("Graph traversal failed to meaningful value");
        }
    }

    Ok(())
}
