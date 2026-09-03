//! Per-view metrics are noted in memory on ingest and flushed to `_00_query`
//! on a timer, so the ingest path never writes the rows the edge transaction
//! is writing at the same moment.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use serde_json::Value;
use ssp_node::ports::{Db, DbError};
use ssp_node::view_metrics::ViewMetrics;
use ssp_node::node::{flush_view_metrics, note_view_metrics};
use surrealdb::engine::local::{Db as MemEngine, Mem};
use surrealdb::Surreal;
use tokio::sync::RwLock;

struct MemDb(Arc<Surreal<MemEngine>>, Arc<AtomicUsize>);

#[async_trait::async_trait]
impl Db for MemDb {
    async fn query(&self, surql: &str, binds: &[(&str, Value)]) -> Result<Vec<Value>, DbError> {
        if surql.contains("rowCount") {
            self.1.fetch_add(1, Ordering::SeqCst);
        }
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

async fn setup() -> (MemDb, Arc<Surreal<MemEngine>>, Arc<AtomicUsize>) {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db.use_ns("t").use_db("t").await.unwrap();
    let raw = Arc::new(db);
    raw.query("CREATE _00_query:v1 SET clientId = 'c', auth_id = 'user:a', rowCount = 0, updateCount = 0;")
        .await
        .unwrap();
    let writes = Arc::new(AtomicUsize::new(0));
    (MemDb(Arc::clone(&raw), Arc::clone(&writes)), raw, writes)
}

async fn row(raw: &Surreal<MemEngine>) -> Value {
    raw.query("SELECT rowCount, updateCount, materializationP90 FROM ONLY _00_query:v1")
        .await
        .unwrap()
        .take::<surrealdb::types::Value>(0)
        .unwrap()
        .into_json_value()
}

#[tokio::test]
async fn many_ingests_flush_as_one_write_and_a_quiet_flush_writes_nothing() {
    let (db, raw, writes) = setup().await;
    let metrics: ViewMetrics = RwLock::new(Default::default());
    for i in 0..100 {
        note_view_metrics(&metrics, vec![i + 1], vec!["_00_query:v1".to_string()], 3.0).await;
    }
    assert_eq!(writes.load(Ordering::SeqCst), 0, "noting never touches the DB");

    let flushed = flush_view_metrics(&db, &metrics).await;
    assert_eq!(flushed, 1);
    assert_eq!(writes.load(Ordering::SeqCst), 1, "100 ingests, one UPDATE");
    let r = row(&raw).await;
    assert_eq!(r["rowCount"], 100);
    assert_eq!(r["updateCount"], 100);
    assert_eq!(r["materializationP90"], 3.0);

    let again = flush_view_metrics(&db, &metrics).await;
    assert_eq!(again, 0, "nothing dirty, nothing written");
    assert_eq!(writes.load(Ordering::SeqCst), 1);
}
