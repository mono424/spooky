use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use std::collections::HashMap;
use anyhow::Result;
use surrealdb::types::Value;
use surrealdb::IndexedResults;

// Hilfsfunktion: Nimmt &str (Referenz), um unnötiges Klonen zu vermeiden
fn parse_vars(vars: Option<&str>) -> Result<HashMap<String, serde_json::Value>> {
    match vars {
        // Wenn String da ist und nicht leer -> Parsen
        Some(v) if !v.is_empty() => Ok(serde_json::from_str(v)?),
        // Wenn None oder leer -> Leere Map
        _ => Ok(HashMap::new()),
    }
}

// Hauptfunktion: Nimmt Option<String>, weil das gut für FFI/Bridge ist
pub async fn query(db: &Surreal<Any>, sql: String, vars: Option<String>) -> Result<String> {
    let mut query = db.query(sql);

    // Hier wandeln wir Option<String> in Option<&str> um mit .as_deref()
    let parsed_vars = parse_vars(vars.as_deref())?;
    
    for (key, value) in parsed_vars {
        query = query.bind((key, value));
    }

    let mut response: IndexedResults = query.await?;
    
    // Check available results count
    let num = response.results.len();
    let mut output = Vec::with_capacity(num);

    for i in 0..num {
        match response.take::<Value>(i) {
            Ok(v3_val) => {
                 // Convert Surreal Value to serde_json::Value
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

// Transaction nutzt auch Option<String> für Konsistenz
pub async fn transaction(db: &Surreal<Any>, statements: String, vars: Option<String>) -> Result<String> {
    let stmts: Vec<String> = serde_json::from_str(&statements)?;
    let joined = stmts.join("; ");
    let sql = format!("BEGIN TRANSACTION; {}; COMMIT TRANSACTION;", joined);
    // Hier geben wir vars einfach weiter
    query(db, sql, vars).await
}

// Die Helfer bleiben gleich...
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