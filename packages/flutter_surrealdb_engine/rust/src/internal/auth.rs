use surrealdb::Surreal;

use anyhow::Result;
use surrealdb::opt::auth::{Root, Namespace, Database, Record, Token};

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum AuthCredentials {
    // Specific variants first to avoid Root capturing everything
    Record { ns: String, db: String, access: String, #[serde(flatten)] params: serde_json::Value },
    RecordLegacy { ns: String, db: String, sc: String, #[serde(flatten)] params: serde_json::Value },
    Database { ns: String, db: String, user: String, pass: String },
    Namespace { ns: String, user: String, pass: String },
    Root { user: String, pass: String },
}

pub async fn signup<C: surrealdb::Connection>(db: &Surreal<C>, credentials: String) -> Result<String> {
    let creds: AuthCredentials = serde_json::from_str(&credentials)?;
    let token: Token = match creds {
        AuthCredentials::Root { .. } | AuthCredentials::Namespace { .. } | AuthCredentials::Database { .. } => {
            anyhow::bail!("Signup is only supported for Record (Scope) users. Use DEFINE USER query for other levels.");
        },
        AuthCredentials::Record { ns, db: db_name, access, params } => {
             db.signup(Record { namespace: ns, database: db_name, access: access, params }).await?
        },
        AuthCredentials::RecordLegacy { ns, db: db_name, sc, params } => {
             db.signup(Record { namespace: ns, database: db_name, access: sc, params }).await?
        },
    };
    Ok(serde_json::to_string(&token)?)
}

pub async fn signin<C: surrealdb::Connection>(db: &Surreal<C>, credentials: String) -> Result<String> {
    let creds: AuthCredentials = serde_json::from_str(&credentials)?;
    let token: Token = match creds {
        AuthCredentials::Root { user, pass } => {
            db.signin(Root { username: user, password: pass }).await?
        },
        AuthCredentials::Namespace { ns, user, pass } => {
            db.signin(Namespace { namespace: ns, username: user, password: pass }).await?
        },
        AuthCredentials::Database { ns, db: db_name, user, pass } => {
            db.signin(Database { namespace: ns, database: db_name, username: user, password: pass }).await?
        },
        AuthCredentials::Record { ns, db: db_name, access, params } => {
             db.signin(Record { namespace: ns, database: db_name, access: access, params }).await?
        },
        AuthCredentials::RecordLegacy { ns, db: db_name, sc, params } => {
             db.signin(Record { namespace: ns, database: db_name, access: sc, params }).await?
        },
    };
    Ok(serde_json::to_string(&token)?)
}

pub async fn authenticate<C: surrealdb::Connection>(db: &Surreal<C>, token: String) -> Result<()> {
    // We expect a serialized Token string (JSON)
    let token_obj: Token = serde_json::from_str(&token)?;
    db.authenticate(token_obj).await?;
    Ok(())
}

pub async fn invalidate<C: surrealdb::Connection>(db: &Surreal<C>) -> Result<()> {
    db.invalidate().await?;
    Ok(())
}
