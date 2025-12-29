use surrealdb::Surreal;

use std::collections::HashMap;
use anyhow::Result;
use crate::frb_generated::StreamSink;
use surrealdb::types::Value;
// use surrealdb::IndexedResults;

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
pub async fn query<C: surrealdb::Connection>(db: &Surreal<C>, sql: String, vars: Option<String>) -> Result<String> {
    let mut query = db.query(sql);

    // Hier wandeln wir Option<String> in Option<&str> um mit .as_deref()
    let parsed_vars = parse_vars(vars.as_deref())?;
    
    for (key, value) in parsed_vars {
        query = query.bind((key, value));
    }

    let mut response: surrealdb::IndexedResults = query.await?;
    
    // In v3, IndexedResults might be a Map <usize, QueryResult> or Vec.
    // Error suggested it yields tuples (usize, ...).
    // Let's assume it is a Map or just iterate values if possible.
    // If it is a BTreeMap<usize, QueryResult>, into_iter yields (usize, QueryResult).
    
    let results = response.results;
    let mut output = Vec::with_capacity(results.len());

    for (_i, result) in results {
        let value_result = result.1; // Attempt to access result via tuple index

        match value_result {
            Ok(v3_val) => {
                output.push(v3_val.into_json_value());
            },
            Err(e) => {
                 output.push(serde_json::json!({ "error": e.to_string() }));
            }
        }
    }
    
    Ok(serde_json::to_string(&output)?)
}

// Transaction nutzt auch Option<String> für Konsistenz
pub async fn transaction<C: surrealdb::Connection>(db: &Surreal<C>, statements: String, vars: Option<String>) -> Result<String> {
    let stmts: Vec<String> = serde_json::from_str(&statements)?;
    let joined = stmts.join("; ");
    let sql = format!("BEGIN TRANSACTION; {}; COMMIT TRANSACTION;", joined);
    // Hier geben wir vars einfach weiter
    query(db, sql, vars).await
}

// Die Helfer bleiben gleich...
pub async fn query_begin<C: surrealdb::Connection>(db: &Surreal<C>) -> Result<()> {
    db.query("BEGIN TRANSACTION").await?;
    Ok(())
}

pub async fn query_commit<C: surrealdb::Connection>(db: &Surreal<C>) -> Result<()> {
    db.query("COMMIT TRANSACTION").await?;
    Ok(())
}

pub async fn query_cancel<C: surrealdb::Connection>(db: &Surreal<C>) -> Result<()> {
    db.query("CANCEL TRANSACTION").await?;
    Ok(())
}