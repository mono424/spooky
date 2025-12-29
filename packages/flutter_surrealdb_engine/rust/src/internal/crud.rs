use surrealdb::Surreal;
use anyhow::Result;
// use surrealdb::sql::Value as SValue;
// use surrealdb::sql::Object as SObject;

// Helper: Serialize input to JSON string
// fn to_json(data: impl serde::Serialize) -> Result<String> {
//     Ok(serde_json::to_string(&data)?)
// }

// Reference: mirrors the `select` method in JS SDK
pub async fn select<C: surrealdb::Connection>(db: &Surreal<C>, resource: String) -> Result<String> {
    // Select using query to handle serialization consistently
    let sql = format!("SELECT * FROM {}", resource);
    super::queries::query(db, sql, None).await
}

// Reference: mirrors the `create` method in JS SDK
pub async fn create<C: surrealdb::Connection>(db: &Surreal<C>, resource: String, data: Option<String>) -> Result<String> {
    let sql = format!("CREATE {} CONTENT $data", resource);
    
    let mut q = db.query(sql);
    
    if let Some(d) = data {
         if !d.is_empty() {
             let content: serde_json::Value = serde_json::from_str(&d)?;
             q = q.bind(("data", content));
         } else {
             q = q.bind(("data", serde_json::Value::Null));
         }
    } else {
         q = q.bind(("data", serde_json::Value::Null));
    }
    
    let mut response = q.await?;
    
    // Return the first result (created record) as JSON
    // We use the same generic Value approach to ensure safe serialization
    // Return the first result (created record) as JSON
    // We use the same generic Value approach to ensure safe serialization
    // In v2, results field is private, so we blindly take(0). 
    // If it fails (index out of bounds or error), we return empty.
    
    // Attempt to take the first result result
    match response.take::<surrealdb::types::Value>(0) {
        Ok(result) => {
            let json = result.into_json_value();
            Ok(json.to_string())
        },
        Err(_) => Ok("{}".to_string())
    }
}

// Reference: mirrors the `update` method in JS SDK
pub async fn update<C: surrealdb::Connection>(db: &Surreal<C>, resource: String, data: Option<String>) -> Result<String> {
    let sql = format!("UPDATE {} CONTENT $data RETURN NONE", resource);
    
    let mut q = db.query(sql);
    if let Some(d) = data {
        if !d.is_empty() {
            let content: serde_json::Value = serde_json::from_str(&d)?;
            q = q.bind(("data", content));
        } else {
            q = q.bind(("data", serde_json::Value::Null));
        }
    } else {
        q = q.bind(("data", serde_json::Value::Null));
    }

    q.await?;
    Ok("{}".to_string())
}

// Reference: mirrors the `merge` method in JS SDK
pub async fn merge<C: surrealdb::Connection>(db: &Surreal<C>, resource: String, data: Option<String>) -> Result<String> {
    let sql = format!("UPDATE {} MERGE $data RETURN NONE", resource);
    
    let mut q = db.query(sql);
    if let Some(d) = data {
        if !d.is_empty() {
            let content: serde_json::Value = serde_json::from_str(&d)?;
            q = q.bind(("data", content));
        } else {
            q = q.bind(("data", serde_json::Value::Null));
        }
    } else {
        q = q.bind(("data", serde_json::Value::Null));
    }

    q.await?;
    Ok("{}".to_string())
}

// Reference: mirrors the `delete` method in JS SDK
pub async fn delete<C: surrealdb::Connection>(db: &Surreal<C>, resource: String) -> Result<String> {
    // Delete via query to maintain consistency
    let sql = format!("DELETE {} RETURN NONE", resource);
    db.query(sql).await?;
    Ok("{}".to_string())
}
