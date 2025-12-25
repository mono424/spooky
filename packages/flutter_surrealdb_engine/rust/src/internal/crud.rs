use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use anyhow::Result;
// use surrealdb::sql::Value as SValue;
// use surrealdb::sql::Object as SObject;

// Helper: Serialize input to JSON string
// fn to_json(data: impl serde::Serialize) -> Result<String> {
//     Ok(serde_json::to_string(&data)?)
// }

// Reference: mirrors the `select` method in JS SDK
pub async fn select(db: &Surreal<Any>, resource: String) -> Result<String> {
    // Select using query to handle serialization consistently
    let sql = format!("SELECT * FROM {}", resource);
    super::query::query(db, sql, None).await
}

// Reference: mirrors the `create` method in JS SDK
pub async fn create(db: &Surreal<Any>, resource: String, data: Option<String>) -> Result<String> {
    // WORKAROUND: RETURN NONE to avoid serialization issues with hidden Thing structs in surrealkv
    let sql = format!("CREATE {} CONTENT $data RETURN NONE", resource);
    
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
    
    q.await?; // Execute
    Ok("{}".to_string())
}
/*
    let mut response = q.await?;
    // let result: Vec<SValue> = response.take(0)?;
    // Try taking raw value
    let result: SValue = response.take(0)?;
    // If it's an array, serialize it. If it's a single object (unlikely for create unless single?), wrap.
    // Usually CREATE returns generic array.
    to_json(result)
*/

// Reference: mirrors the `update` method in JS SDK
pub async fn update(db: &Surreal<Any>, resource: String, data: Option<String>) -> Result<String> {
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
pub async fn merge(db: &Surreal<Any>, resource: String, data: Option<String>) -> Result<String> {
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
pub async fn delete(db: &Surreal<Any>, resource: String) -> Result<String> {
    // Delete via query to maintain consistency
    let sql = format!("DELETE {} RETURN NONE", resource);
    db.query(sql).await?;
    Ok("{}".to_string())
}
