//! Executable proof of the generic runtime seam: drive the portable host
//! through hyper, snapshot to disk, EVICT the in-memory circuit, mutate the DB
//! behind its back, then bootstrap from disk — and confirm the restored circuit
//! caught up to the post-snapshot writes. State survives eviction across a
//! non-axum ingress and a disk CircuitStore, with zero core changes.

use std::sync::Arc;

use bytes::Bytes;
use http_body_util::Full;
use hyper::{Request, Response};
use serde_json::Value;
use surrealdb::engine::local::Mem;
use surrealdb::Surreal;

use ssp_portable::{handle, PortableHost};

const SECRET: &str = "test-secret";

fn get(node_path: &str) -> Request<Full<Bytes>> {
    Request::builder()
        .method("GET")
        .uri(node_path)
        .header("Authorization", format!("Bearer {SECRET}"))
        .body(Full::new(Bytes::new()))
        .unwrap()
}

fn post(node_path: &str, body: Value) -> Request<Full<Bytes>> {
    Request::builder()
        .method("POST")
        .uri(node_path)
        .header("Authorization", format!("Bearer {SECRET}"))
        .body(Full::new(Bytes::from(serde_json::to_vec(&body).unwrap())))
        .unwrap()
}

async fn json_body(resp: Response<Full<Bytes>>) -> Value {
    use http_body_util::BodyExt;
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap_or(Value::Null)
}

#[tokio::test]
async fn state_survives_eviction_via_disk_snapshot_and_catchup() {
    let dir = tempfile::tempdir().unwrap();
    let db = Arc::new(Surreal::new::<Mem>(()).await.unwrap());
    db.use_ns("spooky").use_db("spooky").await.unwrap();

    // Upstream schema + two versioned rows (as a change-feed would have pushed).
    db.query("DEFINE TABLE thread SCHEMALESS PERMISSIONS FOR select FULL;")
        .await
        .unwrap();
    db.query(
        "CREATE thread:1 SET title = 'a', _00_rv = 1; \
         CREATE thread:2 SET title = 'b', _00_rv = 2;",
    )
    .await
    .unwrap();

    // Cold boot: rebuild from DB. Register a live view over `thread` via hyper.
    let mut host = PortableHost::new(db.clone(), dir.path(), SECRET, Some(3600)).await;
    let reg = post(
        "/view/register",
        serde_json::json!({
            "id": "v1", "surql": "SELECT * FROM thread", "clientId": "c1",
            "ttl": "30m", "lastActiveAt": "2024-01-01T00:00:00Z"
        }),
    );
    let r = handle(&host.node, reg).await;
    assert_eq!(r.status(), 200, "view register via hyper");

    // Two rows visible in the view.
    let dbg = json_body(handle(&host.node, get("/debug/view/v1")).await).await;
    assert_eq!(dbg["cache_size"].as_u64(), Some(2), "initial view: {dbg:?}");

    // Persist a snapshot to disk (the CircuitCheckpoint the host would run).
    host.runtime.checkpoint().await;
    assert!(dir.path().join("snapshot.json").exists(), "disk snapshot written");

    // Upstream writes AFTER the snapshot (higher _00_rv), invisible to the
    // evicted circuit until catch-up.
    db.query("CREATE thread:3 SET title = 'c', _00_rv = 3;")
        .await
        .unwrap();

    // EVICT: drop + reconstruct + bootstrap (restore from disk + _00_rv catch-up).
    host.evict().await;

    // The restored view is back AND caught up to the post-snapshot row.
    let dbg = json_body(handle(&host.node, get("/debug/view/v1")).await).await;
    assert_eq!(
        dbg["cache_size"].as_u64(),
        Some(3),
        "view survived eviction and caught up: {dbg:?}"
    );

    // Sanity: the node reports Ready over hyper.
    let health = handle(&host.node, get("/health")).await;
    assert_eq!(health.status(), 200);
}
