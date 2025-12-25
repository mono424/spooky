use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use std::collections::HashMap;
use anyhow::Result;
// use surrealdb::sql::Value as SValue;
// use surrealdb::sql::{Value, Array};

fn parse_vars(vars: &str) -> Result<HashMap<String, serde_json::Value>> {
    if vars.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(serde_json::from_str(vars)?)
}

// Helper to collect all statement results
async fn collect_results(mut response: surrealdb::Response) -> Result<String> {
    let num = response.num_statements();
    let mut output = Vec::with_capacity(num);

    for i in 0..num {
        // Deserialize as single SValue
        let result: Result<surrealdb::Value, _> = response.take(i);
        match result {
            Ok(val) => {
                // Convert to serde_json::Value immediately
                if let Ok(json_val) = serde_json::to_value(&val) {
                    output.push(json_val);
                } else {
                    output.push(serde_json::Value::Null);
                }
            },
            Err(e) => {
                 eprintln!("Serialization Error skipping statement {}: {}", i, e);
                 output.push(serde_json::Value::Null); 
            }
        }
    }
    
    Ok(serde_json::to_string(&output)?)
}

pub async fn query(db: &Surreal<Any>, sql: String, vars: String) -> Result<String> {
    let mut query = db.query(sql);

    let parsed_vars = parse_vars(&vars)?;
    for (key, value) in parsed_vars {
        query = query.bind((key, value));
    }

    let response = query.await?;
    collect_results(response).await
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
