use surrealdb::Surreal;
use surrealdb::engine::any::Any;
use anyhow::Result;
use surrealdb::opt::auth::{Root, Namespace, Database, Record, Jwt};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
struct AuthCredentials {
    user: Option<String>,
    pass: Option<String>,
    #[serde(rename = "NS")]
    ns: Option<String>,
    #[serde(rename = "DB")]
    db: Option<String>,
    #[serde(rename = "SC")]
    sc: Option<String>,
    #[serde(flatten)]
    extra: std::collections::HashMap<String, Value>,
}

fn to_json(data: impl serde::Serialize) -> Result<String> {
    Ok(serde_json::to_string(&data)?)
}

#[allow(dead_code)]
fn extract_token_from_query_result(res: Vec<Value>) -> Result<String> {
    if res.is_empty() {
        return Err(anyhow::anyhow!("Authentication returned no result"));
    }
    let val = &res[0];
    if let Some(token_web) = val.as_str() {
       return to_json(json!({ "token": token_web }));
    }
    to_json(val) 
}

fn wrap_token(token: Jwt) -> Result<String> {
     // Dart test expects {"token": "..."}
     // We rely on serde serialization of the Jwt struct.
     Ok(serde_json::to_string(&json!({ "token": token }))?)
}

pub async fn signup(db: &Surreal<Any>, credentials: String) -> Result<String> {
    let creds: AuthCredentials = serde_json::from_str(&credentials)?;
    let token: Jwt;
    
    if let Some(sc) = &creds.sc {
        // Scope/Record Auth
        let ns = creds.ns.as_deref().ok_or_else(|| anyhow::anyhow!("NS required for Scope auth"))?;
        let db_name = creds.db.as_deref().ok_or_else(|| anyhow::anyhow!("DB required for Scope auth"))?;
        
        db.use_ns(ns).await?;
        db.use_db(db_name).await?;
        
        // Construct params for the record auth.
        let mut params = creds.extra.clone();
        if let Some(u) = &creds.user { params.insert("username".to_string(), json!(u)); }
        if let Some(p) = &creds.pass { params.insert("password".to_string(), json!(p)); }
        
        // Record struct uses generic P. Using Value::Object(Map) or HashMap directly.
        // Record<'a, P>
        // Check if map is strictly needed or HashMap is fine.
        // Value implements Serialize.
        
        let record_creds = Record {
            namespace: ns,
            database: db_name,
            access: sc, 
            params: params,
        };
        
        token = db.signup(record_creds).await?;
        
    } else {
        // Other auth methods for signup usually imply Root/NS/DB, but standard signup is usually for scoped users (records).
        // SurrealDB v2 might support signing up as other levels?
        // For now, if no SC, return error or try Record with empty access? No.
        return Err(anyhow::anyhow!("Signup requires SC (Scope/Access)"));
    }
    
    wrap_token(token)
}

pub async fn signin(db: &Surreal<Any>, credentials: String) -> Result<String> {
    let creds: AuthCredentials = serde_json::from_str(&credentials)?;
    let token: Jwt;
    
    if let Some(sc) = &creds.sc {
        let ns = creds.ns.as_deref().ok_or_else(|| anyhow::anyhow!("NS required for Scope auth"))?;
        let db_name = creds.db.as_deref().ok_or_else(|| anyhow::anyhow!("DB required for Scope auth"))?;
        
        db.use_ns(ns).await?;
        db.use_db(db_name).await?;
        
        let mut params = creds.extra.clone();
        if let Some(u) = &creds.user { params.insert("username".to_string(), json!(u)); }
        if let Some(p) = &creds.pass { params.insert("password".to_string(), json!(p)); }
        
        let record_creds = Record {
            namespace: ns,
            database: db_name,
            access: sc,
            params: params,
        };
        
        token = db.signin(record_creds).await?;
        
    } else if let (Some(ns), Some(db_name), Some(user), Some(pass)) = (&creds.ns, &creds.db, &creds.user, &creds.pass) {
        token = db.signin(Database {
             namespace: ns,
             database: db_name,
             username: user,
             password: pass,
        }).await?;
    } else if let (Some(ns), Some(user), Some(pass)) = (&creds.ns, &creds.user, &creds.pass) {
        token = db.signin(Namespace {
            namespace: ns,
            username: user,
            password: pass,
        }).await?;
    } else if let (Some(user), Some(pass)) = (&creds.user, &creds.pass) {
        token = db.signin(Root {
            username: user,
            password: pass,
        }).await?;
    } else {
        return Err(anyhow::anyhow!("Invalid authentication credentials format"));
    }
    
    wrap_token(token)
}

pub async fn authenticate(db: &Surreal<Any>, token: String) -> Result<()> {
    db.authenticate(token).await?;
    Ok(())
}

pub async fn invalidate(db: &Surreal<Any>) -> Result<()> {
    db.invalidate().await?;
    Ok(())
}
