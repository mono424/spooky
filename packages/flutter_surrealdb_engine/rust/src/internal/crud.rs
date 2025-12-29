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
    super::query::query(db, sql, None).await
}

// Reference: mirrors the `create` method in JS SDK
pub async fn create<C: surrealdb::Connection>(db: &Surreal<C>, resource: String, data: Option<String>) -> Result<String> {
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
