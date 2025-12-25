use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use std::collections::HashMap;
use anyhow::Result;
use surrealdb::types::Value;
use surrealdb::IndexedResults;

fn parse_vars(vars: &str) -> Result<HashMap<String, serde_json::Value>> {
    if vars.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(serde_json::from_str(vars)?)
}

pub async fn query(db: &Surreal<Any>, sql: String, vars: String) -> Result<String> {
    let mut query = db.query(sql);

    let parsed_vars = parse_vars(&vars)?;
    for (key, value) in parsed_vars {
        query = query.bind((key, value));
    }

    let mut response: IndexedResults = query.await?;
    
    // Check available results count
    let num = response.results.len();
    let mut output = Vec::with_capacity(num);

    for i in 0..num {
        // take(i) returns Result<Value>
        match response.take::<Value>(i) {
            Ok(v3_val) => {
                 // Convert v3 Value to serde_json::Value for output
                 // v3 Value likely implements Serialize/Deserialize or conversion
                 // If it implements Serialize, we can just push it to a collection that will be serialized?
                 // But we want a uniform output format.
                 // Let's assume v3 Value implements Serialize.
                 if let Ok(json_val) = serde_json::to_value(&v3_val) {
                     output.push(json_val);
                 } else {
                     output.push(serde_json::Value::Null);
                 }
            },
            Err(e) => {
                 eprintln!("Error taking result {}: {}", i, e);
                 output.push(serde_json::Value::Null);
            }
        }
    }
    
    Ok(serde_json::to_string(&output)?)
}

pub async fn transaction(db: &Surreal<Any>, statements: String, vars: String) -> Result<String> {
    let stmts: Vec<String> = serde_json::from_str(&statements)?;
    let joined = stmts.join("; ");
    let sql = format!("BEGIN TRANSACTION; {}; COMMIT TRANSACTION;", joined);
    query(db, sql, vars).await
}

pub async fn query_begin(db: &Surreal<Any>) -> Result<()> {
    db.query("BEGIN TRANSACTION").await?;
    Ok(())
}

pub async fn query_commit(db: &Surreal<Any>) -> Result<()> {
    db.query("COMMIT TRANSACTION").await?;
    Ok(())
}

pub async fn query_cancel(db: &Surreal<Any>) -> Result<()> {
    db.query("CANCEL TRANSACTION").await?;
    Ok(())
}
