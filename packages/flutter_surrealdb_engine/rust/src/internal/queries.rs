use surrealdb::Surreal;
use anyhow::Result;
use crate::internal::converter; 

// Hauptfunktion: Nimmt Option<String>, weil das gut für FFI/Bridge ist
// Classic Query: Binds JSON as generic Map (No Value conversion)
pub async fn query<C: surrealdb::Connection>(db: &Surreal<C>, sql: String, vars: Option<String>) -> Result<String> {
    let mut query = db.query(sql);
    let parsed_vars = converter::parse_vars(vars.as_deref())?;
    
    for (key, value) in parsed_vars {
        query = query.bind((key, value));
    }

    let mut response: surrealdb::IndexedResults = query.await?;
    process_results(response)
}

// Typed Query: Uses json_to_surreal middleware for RecordId conversion
pub async fn query_typed<C: surrealdb::Connection>(db: &Surreal<C>, sql: String, vars: Option<String>) -> Result<String> {
    let mut query = db.query(sql);

    // Hier wandeln wir Option<String> in Option<&str> um mit .as_deref()
    let parsed_vars = converter::parse_vars(vars.as_deref())?;
    
    for (key, value) in parsed_vars {
        let surreal_val = converter::json_to_surreal(value);
        query = query.bind((key, surreal_val));
    }

    let mut response: surrealdb::IndexedResults = query.await?;
    process_results(response)
}

// Helper to avoid duplication in result processing
fn process_results(response: surrealdb::IndexedResults) -> Result<String> {
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
    // Uses classic query by default
    query(db, sql, vars).await
}

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