//! `ssp_node::tables` over a real embedded SurrealDB via the `Db` port.
//! Proves the per-user `_00_list_ref` DDL + cleanup SQL run correctly through
//! the flattened-JSON port interface (was validated only inside the VM shell).

use std::sync::Arc;

use serde_json::Value;
use ssp_node::ports::{Db, DbError};
use ssp_node::tables;
use ssp_protocol::RefMode;
use surrealdb::engine::local::{Db as MemEngine, Mem};
use surrealdb::Surreal;

struct MemDb(Arc<Surreal<MemEngine>>);

#[async_trait::async_trait]
impl Db for MemDb {
    async fn query(&self, surql: &str, binds: &[(&str, Value)]) -> Result<Vec<Value>, DbError> {
        let mut q = self.0.query(surql);
        for (name, value) in binds {
            q = q.bind(((*name).to_string(), value.clone()));
        }
        let mut response = q.await.map_err(|e| DbError::Transport(e.to_string()))?;
        let n = response.num_statements();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let val: surrealdb::types::Value =
                response.take(i).map_err(|e| DbError::Query(e.to_string()))?;
            out.push(val.into_json_value());
        }
        Ok(out)
    }
    async fn version(&self) -> Result<String, DbError> {
        Ok("mem".into())
    }
}

async fn mem() -> (Arc<dyn Db>, Arc<Surreal<MemEngine>>) {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db.use_ns("t").use_db("t").await.unwrap();
    let raw = Arc::new(db);
    (Arc::new(MemDb(Arc::clone(&raw))), raw)
}

async fn table_exists(raw: &Surreal<MemEngine>, name: &str) -> bool {
    let info = raw
        .query("INFO FOR DB")
        .await
        .unwrap()
        .take::<surrealdb::types::Value>(0)
        .unwrap()
        .into_json_value();
    info.get("tables")
        .and_then(|t| t.as_object())
        .map(|m| m.contains_key(name))
        .unwrap_or(false)
}

#[test]
fn list_ref_table_names() {
    assert_eq!(tables::list_ref_table(RefMode::Single, "user:alice"), "_00_list_ref");
    let dedicated = tables::list_ref_table(RefMode::Dedicated, "user:alice");
    assert!(dedicated.starts_with("_00_list_ref"), "dedicated: {dedicated}");
}

#[tokio::test]
async fn single_mode_defines_no_per_user_tables() {
    let (db, raw) = mem().await;
    tables::ensure_user_tables(db.as_ref(), RefMode::Single, "user:alice")
        .await
        .unwrap();
    // No _00_list_ref_user_* table created in single mode.
    let info = raw
        .query("INFO FOR DB")
        .await
        .unwrap()
        .take::<surrealdb::types::Value>(0)
        .unwrap()
        .into_json_value();
    let any_user = info
        .get("tables")
        .and_then(|t| t.as_object())
        .map(|m| m.keys().any(|k| k.starts_with("_00_list_ref_user_")))
        .unwrap_or(false);
    assert!(!any_user);
}

#[tokio::test]
async fn dedicated_mode_defines_and_drops_per_user_table() {
    let (db, raw) = mem().await;
    let tbl = tables::list_ref_table(RefMode::Dedicated, "user:alice");

    tables::ensure_user_tables(db.as_ref(), RefMode::Dedicated, "user:alice")
        .await
        .unwrap();
    assert!(table_exists(&raw, &tbl).await, "{tbl} should exist after ensure");

    // Idempotent (OVERWRITE) — second call is fine.
    tables::ensure_user_tables(db.as_ref(), RefMode::Dedicated, "user:alice")
        .await
        .unwrap();

    tables::drop_user_tables(db.as_ref(), RefMode::Dedicated, "user:alice")
        .await
        .unwrap();
    assert!(!table_exists(&raw, &tbl).await, "{tbl} should be gone after drop");
}

#[tokio::test]
async fn drop_if_unused_keeps_table_with_live_query() {
    let (db, raw) = mem().await;
    let tbl = tables::list_ref_table(RefMode::Dedicated, "user:bob");
    tables::ensure_user_tables(db.as_ref(), RefMode::Dedicated, "user:bob")
        .await
        .unwrap();

    // A live query for bob → table must survive.
    raw.query("CREATE _00_query SET auth_id = 'user:bob'").await.unwrap();
    tables::drop_user_table_if_unused(db.as_ref(), RefMode::Dedicated, "user:bob")
        .await
        .unwrap();
    assert!(table_exists(&raw, &tbl).await, "table kept while a query is live");

    // Remove the query → now it should be dropped.
    raw.query("DELETE _00_query").await.unwrap();
    tables::drop_user_table_if_unused(db.as_ref(), RefMode::Dedicated, "user:bob")
        .await
        .unwrap();
    assert!(!table_exists(&raw, &tbl).await, "table dropped once no query remains");
}

#[tokio::test]
async fn drop_orphaned_removes_only_dead_owners() {
    let (db, raw) = mem().await;
    tables::ensure_user_tables(db.as_ref(), RefMode::Dedicated, "user:live")
        .await
        .unwrap();
    tables::ensure_user_tables(db.as_ref(), RefMode::Dedicated, "user:dead")
        .await
        .unwrap();
    // Only `live` has a registered query.
    raw.query("CREATE _00_query SET auth_id = 'user:live'").await.unwrap();

    tables::drop_orphaned_user_tables(db.as_ref(), RefMode::Dedicated)
        .await
        .unwrap();

    assert!(table_exists(&raw, &tables::list_ref_table(RefMode::Dedicated, "user:live")).await);
    assert!(!table_exists(&raw, &tables::list_ref_table(RefMode::Dedicated, "user:dead")).await);
}
