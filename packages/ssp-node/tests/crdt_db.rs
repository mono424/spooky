//! `ssp_node::crdt` over a real embedded SurrealDB via the `Db` port. Proves
//! the Loro merge + `_00_crdt` read-modify-write round-trips through the port
//! (record ids via `type::record($tb,$key)` binds) — was VM-shell-only before.

use std::sync::Arc;

use loro::LoroDoc;
use serde_json::Value;
use ssp_node::crdt::{ApplyRequest, CrdtAllowList, CrdtCache};
use ssp_node::ports::{Db, DbError};
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

/// A Loro update: set `text` = `val` on the doc's root map, exported as an
/// update from empty (base64) — the client wire format `/crdt/apply` expects.
fn make_update(val: &str) -> String {
    use base64::Engine;
    let doc = LoroDoc::new();
    doc.get_map("root").insert("text", val).unwrap();
    doc.commit();
    let bytes = doc
        .export(loro::ExportMode::all_updates())
        .expect("export update");
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

async fn crdt_column(raw: &Surreal<MemEngine>, id: &str) -> Value {
    raw.query(format!("SELECT VALUE _00_crdt FROM ONLY {id}"))
        .await
        .unwrap()
        .take::<surrealdb::types::Value>(0)
        .unwrap()
        .into_json_value()
}

#[tokio::test]
async fn apply_persists_snapshot_and_bumps_rev() {
    let raw = Surreal::new::<Mem>(()).await.unwrap();
    raw.use_ns("t").use_db("t").await.unwrap();
    raw.query("CREATE thread:1 SET title = 'hi';").await.unwrap();
    let raw = Arc::new(raw);
    let db = MemDb(Arc::clone(&raw));

    let cache = CrdtCache::new(16, CrdtAllowList::permissive());

    // First apply → rev 1, snapshot persisted under _00_crdt.body.
    let r = cache
        .apply(
            &db,
            &ApplyRequest {
                table: "thread".into(),
                record_id: "thread:1".into(),
                field: "body".into(),
                update: make_update("hello"),
                peer: "42".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(r.rev, 1);

    let col = crdt_column(&raw, "thread:1").await;
    assert_eq!(col["body"]["rev"], 1);
    assert_eq!(col["body"]["lastPeer"], "42");
    assert!(col["body"]["snapshot"].as_str().is_some(), "snapshot stored");

    // Second apply → rev 2 (read-modify-write bumped it).
    let r = cache
        .apply(
            &db,
            &ApplyRequest {
                table: "thread".into(),
                record_id: "thread:1".into(),
                field: "body".into(),
                update: make_update("hello world"),
                peer: "43".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!(r.rev, 2);
    assert_eq!(crdt_column(&raw, "thread:1").await["body"]["rev"], 2);
}

#[tokio::test]
async fn allow_list_rejects_unlisted_field() {
    let raw = Surreal::new::<Mem>(()).await.unwrap();
    raw.use_ns("t").use_db("t").await.unwrap();
    raw.query("CREATE thread:1 SET title = 'hi';").await.unwrap();
    let raw = Arc::new(raw);
    let db = MemDb(Arc::clone(&raw));

    let mut map = std::collections::HashMap::new();
    map.insert("thread".to_string(), vec!["body".to_string()]);
    let cache = CrdtCache::new(16, CrdtAllowList::from_map(map));

    // "title" is not allow-listed → error, nothing written.
    let err = cache
        .apply(
            &db,
            &ApplyRequest {
                table: "thread".into(),
                record_id: "thread:1".into(),
                field: "title".into(),
                update: make_update("x"),
                peer: "1".into(),
            },
        )
        .await;
    assert!(err.is_err(), "unlisted field rejected");
    assert!(crdt_column(&raw, "thread:1").await.get("title").is_none());
}
