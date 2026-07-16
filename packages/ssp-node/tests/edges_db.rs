//! Validate the ported edge-write path against a REAL embedded SurrealDB via
//! the `Db` port. The key risk in the port was swapping the `$fromN` RecordId
//! bind for `type::thing('_00_query', $fromN)` string binds — this test proves
//! `RELATE`/`UPDATE`/`DELETE` with that form actually create/read/remove
//! `_00_list_ref` rows, i.e. the migration preserved edge-write semantics.

use std::sync::Arc;

use serde_json::Value;
use ssp::circuit::{Circuit, SubqueryDeltaItem, SubqueryOp, ViewDelta};
use ssp_node::edges::{run_edge_writes, EdgeSink, SurrealEdgeSink};
use ssp_node::ports::{Db, DbError, NoopTelemetry};
use ssp_protocol::RefMode;
use surrealdb::engine::local::{Db as MemEngine, Mem};
use surrealdb::Surreal;
use tokio::sync::RwLock;

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

async fn mem() -> Arc<Surreal<MemEngine>> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db.use_ns("t").use_db("t").await.unwrap();
    Arc::new(db)
}

/// Seed the incantation `_00_query:<key>` row + the target record so RELATE has
/// real endpoints, and define `_00_list_ref` as a normal edge table.
async fn seed(raw: &Surreal<MemEngine>, incantation_key: &str, target: &str) {
    // Create the incantation via type::record + bind so arbitrary (non-ident)
    // keys are created correctly — matching how the edge writer references it.
    raw.query("CREATE type::record('_00_query', $k) SET clientId = 'c1', auth_id = 'user:a';")
        .bind(("k", incantation_key.to_string()))
        .await
        .unwrap();
    raw.query(format!("CREATE {target} SET n = 1;")).await.unwrap();
}

async fn edge_count(raw: &Surreal<MemEngine>) -> usize {
    // Count rows in Rust. Tolerate the table not existing yet (SurrealDB v3
    // errors on SELECT from an undefined table) — that just means zero edges.
    match raw.query("SELECT VALUE type::string(id) FROM _00_list_ref").await {
        Ok(mut resp) => resp.take::<Vec<String>>(0).map(|v| v.len()).unwrap_or(0),
        Err(_) => 0,
    }
}

fn delta(query_id: &str, additions: Vec<&str>, removals: Vec<&str>) -> ViewDelta {
    ViewDelta {
        query_id: query_id.to_string(),
        additions: additions.into_iter().map(String::from).collect(),
        removals: removals.into_iter().map(String::from).collect(),
        updates: vec![],
        records: vec![],
        result_hash: String::new(),
        subquery_items: vec![],
        auth_id: "user:a".to_string(),
    }
}

#[tokio::test]
async fn relate_then_delete_roundtrip_through_type_thing_bind() {
    let raw = mem().await;
    seed(&raw, "abc", "user:x").await;
    let db = MemDb(Arc::clone(&raw));
    let circuit = Circuit::new();

    // ADD: RELATE type::thing('_00_query',$from0)->_00_list_ref->user:x
    let d = delta("view:abc", vec!["user:x"], vec![]);
    run_edge_writes(&db, &[&d], &circuit, RefMode::Single, &NoopTelemetry).await;
    assert_eq!(edge_count(&raw).await, 1, "RELATE created one _00_list_ref edge");

    // Verify the edge actually links the incantation → target (the bind resolved).
    let from: Option<String> = raw
        .query("SELECT VALUE type::string(in) FROM _00_list_ref LIMIT 1")
        .await
        .unwrap()
        .take(0)
        .unwrap();
    assert_eq!(from.as_deref(), Some("_00_query:abc"), "edge.in is the incantation");

    // REMOVE: DELETE type::thing('_00_query',$from0)->_00_list_ref WHERE out = user:x
    let d = delta("view:abc", vec![], vec!["user:x"]);
    run_edge_writes(&db, &[&d], &circuit, RefMode::Single, &NoopTelemetry).await;
    assert_eq!(edge_count(&raw).await, 0, "DELETE removed the edge");
}

#[tokio::test]
async fn special_char_incantation_key_survives_bind() {
    // A key that would be unsafe to interpolate raw — proves the type::thing
    // string bind (not literal interpolation) is doing its job.
    let raw = mem().await;
    let key = "weird-key.with:stuff"; // note: query_id tail after last ':'
    // format_incantation_id takes the tail after the LAST ':', so craft a
    // query_id whose tail is a hyphen/dot key.
    let query_id = "view:weird-key.with_stuff";
    seed(&raw, "weird-key.with_stuff", "user:y").await;
    let _ = key;

    let db = MemDb(Arc::clone(&raw));
    let circuit = Circuit::new();
    let d = delta(query_id, vec!["user:y"], vec![]);
    run_edge_writes(&db, &[&d], &circuit, RefMode::Single, &NoopTelemetry).await;
    assert_eq!(edge_count(&raw).await, 1, "edge created despite non-ident key");
}

/// Reproduce the "related data (author/comments) never loads" bug: a `.related()`
/// subquery child edge must be written with a NON-NONE `parent`, because the
/// client pulls subquery children with `WHERE parent IS NOT NONE`
/// (buildSubqueryListRefSelect). Main record `thread:t` and its child `user:u`
/// (alias `author`) go in ONE delta → ONE transaction, so the parent lookup can
/// see the main edge written just before it.
#[tokio::test]
async fn subquery_child_edge_gets_non_none_parent() {
    let raw = mem().await;
    seed(&raw, "q1", "thread:t").await;
    raw.query("CREATE user:u SET n = 1;").await.unwrap();
    let db = MemDb(Arc::clone(&raw));
    let circuit = Circuit::new();

    let d = ViewDelta {
        query_id: "view:q1".to_string(),
        additions: vec!["thread:t".to_string()],
        removals: vec![],
        updates: vec![],
        records: vec![],
        result_hash: String::new(),
        subquery_items: vec![SubqueryDeltaItem {
            id: "user:u".to_string(),
            parent_key: "thread:t".to_string(),
            alias: "author".to_string(),
            op: SubqueryOp::Add,
        }],
        auth_id: "user:a".to_string(),
    };
    run_edge_writes(&db, &[&d], &circuit, RefMode::Single, &NoopTelemetry).await;

    // Two edges: the main thread:t edge + the author user:u edge.
    assert_eq!(edge_count(&raw).await, 2, "main + subquery edge both written");

    // The subquery child edge must have parent IS NOT NONE, or the client's
    // `WHERE parent IS NOT NONE` select drops it and the author never syncs.
    let non_none: Option<i64> = raw
        .query("SELECT VALUE count() FROM _00_list_ref WHERE parent IS NOT NONE GROUP ALL")
        .await
        .unwrap()
        .take(0)
        .unwrap();
    assert_eq!(
        non_none,
        Some(1),
        "the author subquery edge must carry a non-NONE parent (client filters on it)"
    );
}

#[tokio::test]
async fn edge_sink_flush_writes_through_port() {
    let raw = mem().await;
    seed(&raw, "sink", "user:z").await;
    let sink = SurrealEdgeSink {
        db: Arc::new(MemDb(Arc::clone(&raw))),
        processor: Arc::new(RwLock::new(Circuit::new())),
        telemetry: Arc::new(NoopTelemetry),
        mode: RefMode::Single,
    };
    sink.flush(vec![delta("view:sink", vec!["user:z"], vec![])]).await;
    assert_eq!(edge_count(&raw).await, 1, "SurrealEdgeSink wrote through the Db port");
}
