//! Opaque fields must never enter the circuit, and the two content-hash
//! producers must agree.
//!
//! This is the load-bearing property of the `sp00ky:opaque` marker. Before it
//! existed, a field-level `-- @nosync` (or `-- @crdt`) was skipped from the
//! sp00ky ingest event payload but still loaded by the bootstrap `SELECT *` and
//! still cloned into the scheduler replica. The two sides then diverged
//! permanently on the first update, because the replica applies an update with
//! `UPDATE … MERGE` (which cannot remove a key absent from the payload) while
//! the circuit replaces the whole row (which drops it). `spky verify` reported a
//! mismatch that no re-clone could fix, and two SSPs bootstrapped at different
//! times served different row content for the same record.
//!
//! Drives the real `rebuild_from_db` against an embedded SurrealDB over the `Db`
//! port, then hashes both sides with the same `ssp_protocol::snapshot_hash`
//! helpers the scheduler and the SSP use in production.

use std::sync::Arc;

use serde_json::{json, Value};
use ssp::circuit::{Change, ChangeSet, Circuit};
use ssp_node::bootstrap::rebuild_from_db;
use ssp_node::ports::{Db, DbError};
use ssp_protocol::snapshot_hash;
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

/// A deployed schema as the CLI emits it: the opaque marker is baked onto the
/// `DEFINE FIELD` as `COMMENT 'sp00ky:opaque'` (see
/// `schema_builder::add_opaque_field_markers`), which is how the runtime
/// discovers it via `INFO FOR TABLE`.
const SCHEMA: &str = r#"
DEFINE TABLE doc SCHEMAFULL PERMISSIONS FULL;
DEFINE FIELD title ON TABLE doc TYPE string;
DEFINE FIELD thumbnail ON TABLE doc TYPE option<bytes> COMMENT 'sp00ky:opaque';
DEFINE FIELD import_batch ON TABLE doc TYPE option<string> COMMENT 'sp00ky:opaque';
"#;

async fn seeded() -> Arc<Surreal<MemEngine>> {
    let db = Surreal::new::<Mem>(()).await.unwrap();
    db.use_ns("t").use_db("t").await.unwrap();
    db.query(SCHEMA).await.unwrap().check().unwrap();
    db.query(
        "CREATE doc:a SET title = 'A', import_batch = 'b1';
         CREATE doc:b SET title = 'B', import_batch = 'b2';",
    )
    .await
    .unwrap()
    .check()
    .unwrap();
    Arc::new(db)
}

async fn bootstrapped(db: &MemDb) -> Arc<RwLock<Circuit>> {
    let circuit = Arc::new(RwLock::new(Circuit::new()));
    rebuild_from_db(db, &circuit, 100).await.expect("bootstrap");
    circuit
}

#[tokio::test]
async fn bootstrap_does_not_load_opaque_fields_into_the_circuit() {
    let raw = seeded().await;
    let db = MemDb(Arc::clone(&raw));
    let circuit = bootstrapped(&db).await;

    let rows = circuit.read().await.compute_table_hashes();
    assert!(rows.contains_key("doc"), "doc must be hashed: {rows:?}");

    // Inspect the stored rows through the snapshot the circuit serializes.
    let snapshot = circuit.read().await.save().expect("save");
    assert!(
        snapshot.contains("\"title\""),
        "an ordinary field must be loaded"
    );
    assert!(
        !snapshot.contains("import_batch"),
        "an opaque field must never enter the circuit:\n{snapshot}"
    );
    assert!(
        !snapshot.contains("thumbnail"),
        "an opaque field must never enter the circuit:\n{snapshot}"
    );
}

/// The circuit's hash after a cold bootstrap must equal the hash a
/// scheduler-side producer computes over the same rows. Both use
/// `snapshot_hash`, and both must project out opaque fields — a difference in
/// either half is the drift this marker exists to prevent.
#[tokio::test]
async fn circuit_and_replica_style_hashes_agree_after_bootstrap() {
    let raw = seeded().await;
    let db = MemDb(Arc::clone(&raw));
    let circuit = bootstrapped(&db).await;

    // Scheduler side: page upstream with the same OMIT projection the replica
    // clone uses, then hash.
    let upstream: Value = raw
        .query("SELECT * OMIT import_batch, thumbnail FROM doc ORDER BY id")
        .await
        .unwrap()
        .take::<surrealdb::types::Value>(0)
        .unwrap()
        .into_json_value();
    let pairs: Vec<(String, Value)> = upstream
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            let id = row.get("id").unwrap().as_str().unwrap();
            let raw_id = id.strip_prefix("doc:").unwrap_or(id).to_string();
            (raw_id, row.clone())
        })
        .collect();
    let replica_hash = snapshot_hash::hash_table(pairs);

    let circuit_hash = circuit
        .read()
        .await
        .compute_table_hashes()
        .get("doc")
        .cloned()
        .expect("doc hash");

    assert_eq!(
        circuit_hash, replica_hash,
        "circuit and replica-style hashes must agree after a cold bootstrap"
    );
}

/// The regression itself: a bootstrap-loaded row and the same row after an
/// ingest-event update must carry the SAME key set. If bootstrap kept the opaque
/// column, the whole-row replace an ingest update performs would silently drop
/// it and the hash would move even though no user-visible field changed.
#[tokio::test]
async fn an_ingest_update_does_not_change_the_hash_of_unchanged_content() {
    let raw = seeded().await;
    let db = MemDb(Arc::clone(&raw));
    let circuit = bootstrapped(&db).await;

    let before = circuit
        .read()
        .await
        .compute_table_hashes()
        .get("doc")
        .cloned()
        .expect("doc hash");

    // The sp00ky mutation event's `$plain_after` for `doc:a` with only `title`
    // rewritten to the same value: opaque fields are omitted by the generated
    // event (see `sp00ky.rs::is_excluded_field`), `_00_rv` is stamped.
    circuit.write().await.step(ChangeSet {
        changes: vec![Change::update(
            "doc",
            "doc:a",
            json!({ "id": "doc:a", "title": "A", "_00_rv": 2 }),
        )],
    });

    let after = circuit
        .read()
        .await
        .compute_table_hashes()
        .get("doc")
        .cloned()
        .expect("doc hash");

    assert_eq!(
        before, after,
        "an ingest update that changes no hashed content must not move the hash \
         (bootstrap and ingest must agree on the row's key set)"
    );
}

/// A genuine content change must still move the hash — otherwise the assertion
/// above would pass for the wrong reason.
#[tokio::test]
async fn a_real_content_change_still_moves_the_hash() {
    let raw = seeded().await;
    let db = MemDb(Arc::clone(&raw));
    let circuit = bootstrapped(&db).await;

    let before = circuit
        .read()
        .await
        .compute_table_hashes()
        .get("doc")
        .cloned()
        .expect("doc hash");

    circuit.write().await.step(ChangeSet {
        changes: vec![Change::update(
            "doc",
            "doc:a",
            json!({ "id": "doc:a", "title": "A CHANGED", "_00_rv": 2 }),
        )],
    });

    let after = circuit
        .read()
        .await
        .compute_table_hashes()
        .get("doc")
        .cloned()
        .expect("doc hash");

    assert_ne!(before, after, "a real change must move the hash");
}
