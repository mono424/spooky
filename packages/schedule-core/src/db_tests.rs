//! Close-to-e2e engine tests: a real embedded SurrealDB, the REAL table DDL
//! (`include_str!`'d from `apps/cli/src/schedule_tables.surql`, so a DDL change
//! that breaks the engine's SQL fails here rather than on a deploy), and a stub
//! outbox table standing in for the app's job table.
//!
//! There is no runner in these tests. Jobs are "executed" by writing the terminal
//! status a runner would have written, which is exactly the boundary the engine
//! observes across — so these tests cover the engine's half of the contract
//! without simulating HTTP.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use chrono::{Duration, Utc};
use serde_json::{json, Value};
use surrealdb::engine::local::{Db as MemEngine, Mem};
use surrealdb::Surreal;

use crate::db::{ScheduleDb, ScheduleDbError};
use crate::engine::{stamp, EngineConfig, ScheduleEngine};
use crate::ids;
use crate::kill::{JobKill, NoopJobKill};

// --- adapters ---------------------------------------------------------------

struct MemDb(Arc<Surreal<MemEngine>>);

#[async_trait::async_trait]
impl ScheduleDb for MemDb {
    async fn query(
        &self,
        surql: &str,
        binds: &[(&str, Value)],
    ) -> Result<Vec<Value>, ScheduleDbError> {
        let mut q = self.0.query(surql);
        for (name, value) in binds {
            q = q.bind(((*name).to_string(), value.clone()));
        }
        let mut response = q.await.map_err(|e| ScheduleDbError::Transport(e.to_string()))?;
        let n = response.num_statements();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let val: surrealdb::types::Value =
                response.take(i).map_err(|e| ScheduleDbError::Query(e.to_string()))?;
            out.push(val.into_json_value());
        }
        Ok(out)
    }
}

/// Records which jobs were asked to die, so `replace` and kill paths are
/// observable without a runner.
#[derive(Default)]
struct RecordingKill {
    killed: Mutex<Vec<String>>,
    calls: AtomicUsize,
}

#[async_trait::async_trait]
impl JobKill for RecordingKill {
    async fn kill(&self, job_id: &str) -> anyhow::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.killed.lock().unwrap().push(job_id.to_string());
        Ok(())
    }
}

// --- harness ----------------------------------------------------------------

/// The real shipped DDL, so a schema change that breaks the engine's SQL fails
/// in this file rather than on someone's deploy.
const SCHEDULE_TABLES: &str = include_str!("../../../apps/cli/src/schedule_tables.surql");

/// Stand-in for an app's outbox table, matching what `spky add api` generates
/// (including the platform-injected `assignee` / `result` fields).
const OUTBOX_DDL: &str = "\
DEFINE TABLE OVERWRITE job SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD OVERWRITE path ON job TYPE string;
DEFINE FIELD OVERWRITE payload ON job TYPE option<object> FLEXIBLE;
DEFINE FIELD OVERWRITE status ON job TYPE string DEFAULT 'pending';
DEFINE FIELD OVERWRITE retries ON job TYPE int DEFAULT 0;
DEFINE FIELD OVERWRITE max_retries ON job TYPE int DEFAULT 3;
DEFINE FIELD OVERWRITE retry_strategy ON job TYPE string DEFAULT 'linear';
DEFINE FIELD OVERWRITE errors ON job TYPE array<object> DEFAULT [];
DEFINE FIELD OVERWRITE errors[*] ON job TYPE object FLEXIBLE;
DEFINE FIELD OVERWRITE timeout ON job TYPE option<int>;
DEFINE FIELD OVERWRITE delay ON job TYPE option<int>;
DEFINE FIELD OVERWRITE assignee ON job TYPE option<string>;
DEFINE FIELD OVERWRITE result ON job TYPE any;
DEFINE FIELD OVERWRITE created_at ON job TYPE datetime DEFAULT time::now();
DEFINE FIELD OVERWRITE updated_at ON job TYPE datetime VALUE time::now();";

struct Harness {
    engine: ScheduleEngine,
    raw: Arc<Surreal<MemEngine>>,
    kill: Arc<RecordingKill>,
}

async fn harness() -> Harness {
    let db = Surreal::new::<Mem>(()).await.expect("start mem db");
    db.use_ns("test").use_db("test").await.expect("use ns/db");
    let raw = Arc::new(db);

    raw.query(SCHEDULE_TABLES).await.expect("apply scheduling schema");
    raw.query(OUTBOX_DDL).await.expect("apply outbox schema");

    let kill = Arc::new(RecordingKill::default());
    let engine = ScheduleEngine::new(
        Arc::new(MemDb(Arc::clone(&raw))),
        Arc::clone(&kill) as Arc<dyn JobKill>,
        EngineConfig::default(),
    );
    Harness { engine, raw, kill }
}

impl Harness {
    /// Write a schedule row the way `spky deploy` would (spec fields only).
    async fn define_schedule(&self, name: &str, spec: Value) {
        let mut content = spec.as_object().cloned().expect("spec object");
        content.insert("name".into(), json!(name));
        let id = ids::schedule(name);
        self.raw
            .query("CREATE type::record($tb, $key) CONTENT $content")
            .bind(("tb", id.table))
            .bind(("key", id.key))
            .bind(("content", Value::Object(content)))
            .await
            .expect("define schedule");
    }

    /// Runs a statement AND surfaces statement-level errors. `query().await` only
    /// reports transport failures — a rejected write hides in the response until
    /// something takes it, which is exactly how a silently-dropped `errors`
    /// append went unnoticed.
    async fn set(&self, sql: &str) {
        let mut response = self.raw.query(sql).await.expect("statement");
        if let Some(errors) = Some(response.take_errors()).filter(|e| !e.is_empty()) {
            panic!("statement failed: {errors:?}");
        }
    }

    async fn strings(&self, sql: &str) -> Vec<String> {
        self.raw.query(sql).await.expect("query").take(0).expect("take")
    }

    async fn one_string(&self, sql: &str) -> Option<String> {
        self.raw.query(sql).await.expect("query").take(0).expect("take")
    }

    /// Datetime columns come back as datetimes, not strings — compare them via
    /// `type::string(...)` or a boolean predicate rather than `one_string`.
    async fn one_bool(&self, sql: &str) -> Option<bool> {
        self.raw.query(sql).await.expect("query").take(0).expect("take")
    }

    async fn count(&self, sql: &str) -> i64 {
        let n: Option<i64> = self.raw.query(sql).await.expect("query").take(0).expect("take");
        n.unwrap_or(0)
    }

    /// Stand in for the JobRunner finishing a job.
    async fn finish_job(&self, job_id: &str, status: &str, result: Value) {
        let mut response = self
            .raw
            .query("UPDATE type::record($id) SET status = $status, result = $result")
            .bind(("id", job_id.to_string()))
            .bind(("status", status.to_string()))
            .bind(("result", result))
            .await
            .expect("finish job");
        let errors = response.take_errors();
        assert!(errors.is_empty(), "finish_job rejected: {errors:?}");
    }

    async fn fail_job(&self, job_id: &str, reason: &str) {
        let mut response = self
            .raw
            .query(
                "UPDATE type::record($id) SET status = 'failed', \
                 errors = array::append(errors, { code: 500, reason: $reason })",
            )
            .bind(("id", job_id.to_string()))
            .bind(("reason", reason.to_string()))
            .await
            .expect("fail job");
        let errors = response.take_errors();
        assert!(errors.is_empty(), "fail_job rejected: {errors:?}");
    }

    async fn job_ids(&self) -> Vec<String> {
        self.strings("SELECT VALUE type::string(id) FROM job").await
    }

    async fn step_status(&self, step: &str) -> Option<String> {
        self.raw
            .query("SELECT VALUE status FROM ONLY _00_step_run WHERE step = $step LIMIT 1")
            .bind(("step", step.to_string()))
            .await
            .expect("query")
            .take(0)
            .expect("take")
    }

    async fn step_job(&self, step: &str) -> Option<String> {
        self.raw
            .query("SELECT VALUE job_id FROM ONLY _00_step_run WHERE step = $step LIMIT 1")
            .bind(("step", step.to_string()))
            .await
            .expect("query")
            .take(0)
            .expect("take")
    }

    async fn workflow_status(&self) -> Option<String> {
        self.one_string("SELECT VALUE status FROM ONLY _00_workflow_run WHERE true LIMIT 1").await
    }

    /// A second engine over the same database — a replicated scheduler, or a
    /// heal pass racing an ingest event.
    fn rival(&self) -> ScheduleEngine {
        ScheduleEngine::new(
            Arc::new(MemDb(Arc::clone(&self.raw))),
            Arc::new(NoopJobKill),
            EngineConfig::default(),
        )
    }
}

fn every_5m_job() -> Value {
    json!({
        "kind": "job",
        "every_ms": 300_000,
        "target_table": "job",
        "path": "/sync",
    })
}

// --- planning + firing ------------------------------------------------------

#[tokio::test]
async fn plans_then_fires_once_due() {
    let h = harness().await;
    h.define_schedule("nightly", every_5m_job()).await;

    // A freshly deployed schedule has next_fire_at = NONE. Planning happens
    // before firing, but an `every` schedule's first fire is one interval out —
    // so the first sweep plans and does not fire.
    let report = h.engine.tick_pass().await.unwrap();
    assert_eq!(report.planned, 1);
    assert_eq!(report.fired, 0);
    assert!(h.job_ids().await.is_empty());

    h.set("UPDATE _00_schedule:nightly SET next_fire_at = time::now() - 1s").await;
    let report = h.engine.tick_pass().await.unwrap();
    assert_eq!(report.fired, 1);
    assert_eq!(report.spawned, 1);

    assert_eq!(h.job_ids().await.len(), 1, "one atomic job spawned");
    assert_eq!(
        h.one_string("SELECT VALUE status FROM ONLY job WHERE true LIMIT 1").await.as_deref(),
        Some("pending"),
        "the engine creates a pending row and lets the runner take it from there"
    );
    assert_eq!(
        h.one_bool("SELECT VALUE last_fire_at != NONE FROM ONLY _00_schedule:nightly").await,
        Some(true),
        "the fire is stamped on the schedule"
    );
}

#[tokio::test]
async fn a_bad_cron_is_recorded_and_stays_inert() {
    let h = harness().await;
    h.define_schedule(
        "broken",
        json!({ "kind": "job", "cron": "not a cron", "target_table": "job", "path": "/x" }),
    )
    .await;

    let report = h.engine.tick_pass().await.unwrap();
    assert_eq!(report.errored, 1);
    assert_eq!(report.planned, 0);
    let error = h.one_string("SELECT VALUE last_error FROM ONLY _00_schedule:broken").await;
    assert!(error.unwrap().contains("invalid cron"), "the operator can see why");
    assert_eq!(
        h.one_bool("SELECT VALUE next_fire_at = NONE FROM ONLY _00_schedule:broken").await,
        Some(true),
        "an unplannable schedule never fires"
    );

    // One bad schedule must not stop a good one in the same sweep.
    h.define_schedule("fine", every_5m_job()).await;
    let report = h.engine.tick_pass().await.unwrap();
    assert_eq!(report.planned, 1);
    assert_eq!(report.errored, 1);
}

#[tokio::test]
async fn paused_and_disabled_schedules_never_fire() {
    let h = harness().await;
    h.define_schedule("paused-one", every_5m_job()).await;
    h.define_schedule("disabled-one", every_5m_job()).await;
    h.engine.tick_pass().await.unwrap();
    h.set(
        "UPDATE _00_schedule:`paused-one` SET paused = true, next_fire_at = time::now() - 1s; \
         UPDATE _00_schedule:`disabled-one` SET config_disabled = true, \
         next_fire_at = time::now() - 1s",
    )
    .await;

    let report = h.engine.tick_pass().await.unwrap();
    assert_eq!(report.fired, 0);
    assert!(h.job_ids().await.is_empty());

    // Pausing also wins over a queued trigger: the big red button stays red.
    h.set("UPDATE _00_schedule:`paused-one` SET trigger_requested_at = time::now()").await;
    let report = h.engine.tick_pass().await.unwrap();
    assert_eq!(report.fired, 0);
    assert!(h.job_ids().await.is_empty());
}

#[tokio::test]
async fn trigger_fires_now_without_moving_the_cron_clock() {
    let h = harness().await;
    h.define_schedule(
        "nightly",
        json!({ "kind": "job", "cron": "0 3 * * *", "target_table": "job", "path": "/cleanup" }),
    )
    .await;
    h.engine.tick_pass().await.unwrap();
    let planned = h
        .one_string("SELECT VALUE type::string(next_fire_at) FROM ONLY _00_schedule:nightly")
        .await;

    h.set("UPDATE _00_schedule:nightly SET trigger_requested_at = time::now()").await;
    let report = h.engine.tick_pass().await.unwrap();
    assert_eq!(report.fired, 1);
    assert_eq!(report.spawned, 1);
    assert_eq!(
        h.one_string("SELECT VALUE type::string(next_fire_at) FROM ONLY _00_schedule:nightly")
            .await,
        planned,
        "a manual run must not shift the schedule"
    );
    assert_eq!(
        h.one_string("SELECT VALUE trigger FROM ONLY _00_schedule_run WHERE true LIMIT 1")
            .await
            .as_deref(),
        Some("manual"),
        "history distinguishes a manual run from a cron fire"
    );
    assert_eq!(
        h.one_bool("SELECT VALUE trigger_requested_at = NONE FROM ONLY _00_schedule:nightly")
            .await,
        Some(true),
        "the trigger is consumed, not left to fire again"
    );
}

#[tokio::test]
async fn a_second_ticker_cannot_double_fire() {
    let h = harness().await;
    h.define_schedule("nightly", every_5m_job()).await;
    h.engine.tick_pass().await.unwrap();
    h.set("UPDATE _00_schedule:nightly SET next_fire_at = time::now() - 1s").await;

    let rival = h.rival();
    let (a, b) = tokio::join!(h.engine.tick_pass(), rival.tick_pass());
    let fired = a.unwrap().fired + b.unwrap().fired;

    assert_eq!(fired, 1, "the claim CAS lets exactly one ticker win the fire");
    assert_eq!(h.job_ids().await.len(), 1, "and only one job exists");
}

#[tokio::test]
async fn downtime_coalesces_into_a_single_catch_up_fire() {
    let h = harness().await;
    h.define_schedule(
        "hourly",
        json!({ "kind": "job", "cron": "0 * * * *", "target_table": "job", "path": "/tick" }),
    )
    .await;
    h.engine.tick_pass().await.unwrap();
    // Pretend the process slept for a week.
    h.set("UPDATE _00_schedule:hourly SET next_fire_at = time::now() - 7d").await;

    let report = h.engine.tick_pass().await.unwrap();
    assert_eq!(report.fired, 1, "a week of missed slots is one catch-up fire, not 168");
    assert_eq!(h.job_ids().await.len(), 1);
    assert_eq!(
        h.count(
            "SELECT VALUE count() FROM _00_schedule WHERE next_fire_at > time::now() GROUP ALL"
        )
        .await,
        1,
        "the clock is rearmed ahead of now, so the next sweep is quiet"
    );
}

// --- fan-out + concurrency --------------------------------------------------

async fn fan_out_harness(policy: &str) -> Harness {
    let h = harness().await;
    h.set(
        "CREATE connection:alice SET active = true; \
         CREATE connection:bob SET active = true; \
         CREATE connection:carol SET active = false",
    )
    .await;
    h.define_schedule(
        "game-sync",
        json!({
            "kind": "job",
            "every_ms": 300_000,
            "target_table": "job",
            "path": "/syncGames",
            "for_each": "SELECT id FROM connection WHERE active = true",
            "for_each_key": "id",
            "concurrency": policy,
        }),
    )
    .await;
    h.engine.tick_pass().await.unwrap();
    h.set("UPDATE _00_schedule:`game-sync` SET next_fire_at = time::now() - 1s").await;
    h
}

#[tokio::test]
async fn fan_out_spawns_one_job_per_row_with_the_row_in_the_payload() {
    let h = fan_out_harness("skip").await;

    let report = h.engine.tick_pass().await.unwrap();
    assert_eq!(report.fired, 1);
    assert_eq!(report.spawned, 2, "only the two active connections");

    assert_eq!(h.job_ids().await.len(), 2);
    let payload_ids = h.strings("SELECT VALUE payload.row.id FROM job").await;
    assert_eq!(payload_ids.len(), 2);
    assert!(payload_ids.contains(&"connection:alice".to_string()));
    assert!(payload_ids.contains(&"connection:bob".to_string()));

    let keys = h.strings("SELECT VALUE key FROM _00_schedule_run").await;
    assert!(keys.contains(&"connection:alice".to_string()), "runs are keyed per item");
}

#[tokio::test]
async fn skip_suppresses_only_the_key_that_is_still_running() {
    let h = fan_out_harness("skip").await;
    h.engine.tick_pass().await.unwrap();

    // Finish alice's job; leave bob's running.
    let alice_job = h
        .one_string(
            "SELECT VALUE job_id FROM ONLY _00_schedule_run \
             WHERE key = 'connection:alice' LIMIT 1",
        )
        .await
        .unwrap();
    h.finish_job(&alice_job, "success", json!({"synced": 3})).await;
    h.engine.tick_pass().await.unwrap();

    h.set("UPDATE _00_schedule:`game-sync` SET next_fire_at = time::now() - 1s").await;
    let report = h.engine.tick_pass().await.unwrap();

    assert_eq!(report.spawned, 1, "alice is free and runs again");
    assert_eq!(report.skipped, 1, "bob's tick is suppressed, not queued");
    assert_eq!(
        h.count("SELECT VALUE count() FROM _00_schedule_run WHERE status = 'skipped' GROUP ALL")
            .await,
        1,
        "the suppressed tick is recorded so an operator can see it"
    );
}

#[tokio::test]
async fn allow_lets_runs_overlap() {
    let h = fan_out_harness("allow").await;
    h.engine.tick_pass().await.unwrap();
    h.set("UPDATE _00_schedule:`game-sync` SET next_fire_at = time::now() - 1s").await;

    let report = h.engine.tick_pass().await.unwrap();
    assert_eq!(report.spawned, 2);
    assert_eq!(report.skipped, 0);
    assert_eq!(
        h.count("SELECT VALUE count() FROM _00_schedule_run WHERE status = 'running' GROUP ALL")
            .await,
        4,
        "two overlapping runs per key"
    );
}

#[tokio::test]
async fn replace_kills_the_previous_run_for_that_key() {
    let h = fan_out_harness("replace").await;
    h.engine.tick_pass().await.unwrap();
    let first_jobs = h.job_ids().await;
    h.set("UPDATE _00_schedule:`game-sync` SET next_fire_at = time::now() - 1s").await;

    let report = h.engine.tick_pass().await.unwrap();
    assert_eq!(report.replaced, 2);
    assert_eq!(report.spawned, 2);

    let killed = h.kill.killed.lock().unwrap().clone();
    assert_eq!(killed.len(), 2, "both in-flight jobs were killed");
    for job in &first_jobs {
        assert!(killed.contains(job), "{job} should have been killed");
    }
    assert_eq!(
        h.count("SELECT VALUE count() FROM _00_schedule_run WHERE status = 'replaced' GROUP ALL")
            .await,
        2
    );
}

#[tokio::test]
async fn a_failing_for_each_query_is_recorded_not_fatal() {
    let h = harness().await;
    h.define_schedule(
        "broken-fanout",
        json!({
            "kind": "job",
            "every_ms": 300_000,
            "target_table": "job",
            "path": "/x",
            "for_each": "SELECT id FROM",
        }),
    )
    .await;
    h.engine.tick_pass().await.unwrap();
    h.set("UPDATE _00_schedule:`broken-fanout` SET next_fire_at = time::now() - 1s").await;

    let report = h.engine.tick_pass().await.unwrap();
    assert_eq!(report.errored, 1);
    assert!(h.job_ids().await.is_empty());
    assert!(h
        .one_string("SELECT VALUE last_error FROM ONLY _00_schedule:`broken-fanout`")
        .await
        .unwrap()
        .contains("forEach"));
}

// --- healing ----------------------------------------------------------------

/// Fire a plain `every` job schedule once and return its job id.
async fn fire_once(h: &Harness) -> String {
    h.define_schedule("nightly", every_5m_job()).await;
    h.engine.tick_pass().await.unwrap();
    h.set("UPDATE _00_schedule:nightly SET next_fire_at = time::now() - 1s").await;
    h.engine.tick_pass().await.unwrap();
    h.one_string("SELECT VALUE job_id FROM ONLY _00_schedule_run WHERE true LIMIT 1")
        .await
        .expect("a job was spawned")
}

#[tokio::test]
async fn heal_finalizes_a_run_whose_job_finished_without_an_event() {
    let h = harness().await;
    let job = fire_once(&h).await;
    // The runner finished the job, but the ingest event never reached the engine.
    h.finish_job(&job, "success", json!({"ok": true})).await;

    let report = h.engine.tick_pass().await.unwrap();
    assert!(report.healed >= 1);
    assert_eq!(
        h.one_string("SELECT VALUE status FROM ONLY _00_schedule_run WHERE true LIMIT 1")
            .await
            .as_deref(),
        Some("success"),
        "polling caps the delay of a lost event at one sweep"
    );
}

#[tokio::test]
async fn heal_fails_a_run_whose_job_row_vanished() {
    let h = harness().await;
    fire_once(&h).await;
    h.set("DELETE job").await;

    h.engine.tick_pass().await.unwrap();

    assert_eq!(
        h.one_string("SELECT VALUE status FROM ONLY _00_schedule_run WHERE true LIMIT 1")
            .await
            .as_deref(),
        Some("failed"),
        "a run whose job can never complete must not stay `running` forever"
    );
}

#[tokio::test]
async fn a_failed_job_carries_its_error_onto_the_run() {
    let h = harness().await;
    let job = fire_once(&h).await;
    h.fail_job(&job, "backend exploded").await;

    h.engine.tick_pass().await.unwrap();

    assert_eq!(
        h.one_string("SELECT VALUE status FROM ONLY _00_schedule_run WHERE true LIMIT 1")
            .await
            .as_deref(),
        Some("failed")
    );
    assert_eq!(
        h.one_string("SELECT VALUE error.reason FROM ONLY _00_schedule_run WHERE true LIMIT 1")
            .await
            .as_deref(),
        Some("backend exploded")
    );
}

// --- retention --------------------------------------------------------------

#[tokio::test]
async fn prune_drops_old_history_and_spares_live_runs() {
    let h = harness().await;
    let old = stamp(Utc::now() - Duration::days(60));
    h.raw
        .query(
            "CREATE _00_schedule_run:old CONTENT { schedule_name: 'x', key: '', \
             fire_at: <datetime>$old, kind: 'job', status: 'success', \
             finished_at: <datetime>$old }; \
             CREATE _00_schedule_run:recent CONTENT { schedule_name: 'x', key: '', \
             fire_at: time::now(), kind: 'job', status: 'success', finished_at: time::now() }; \
             CREATE _00_schedule_run:live CONTENT { schedule_name: 'x', key: '', \
             fire_at: <datetime>$old, kind: 'job', status: 'running' }",
        )
        .bind(("old", old))
        .await
        .expect("seed history");

    let report = h.engine.tick_pass().await.unwrap();
    assert_eq!(report.pruned, 1);

    let ids = h.strings("SELECT VALUE type::string(id) FROM _00_schedule_run").await;
    assert!(!ids.contains(&"_00_schedule_run:old".to_string()), "aged-out history is gone");
    assert!(ids.contains(&"_00_schedule_run:recent".to_string()));
    assert!(
        ids.contains(&"_00_schedule_run:live".to_string()),
        "an in-flight run is never pruned, however old"
    );
}

// --- workflows --------------------------------------------------------------

/// extract-orders ┐
///                ├→ transform → {notify, archive}
/// extract-users  ┘
fn diamond_workflow() -> Value {
    json!({
        "kind": "workflow",
        "every_ms": 3_600_000,
        "target_table": "job",
        "workflow": {
            "target_table": "job",
            "steps": [
                {"name": "extract-orders", "path": "/exportOrders"},
                {"name": "extract-users", "path": "/exportUsers"},
                {"name": "transform", "path": "/buildReport",
                 "depends_on": ["extract-orders", "extract-users"]},
                {"name": "notify", "path": "/postSlack", "depends_on": ["transform"]},
                {"name": "archive", "path": "/archiveReport", "depends_on": ["transform"]},
            ],
            "on_failure": "halt",
        },
    })
}

async fn workflow_harness(def: Value) -> Harness {
    let h = harness().await;
    h.define_schedule("report", def).await;
    h.engine.tick_pass().await.unwrap();
    h.set("UPDATE _00_schedule:report SET next_fire_at = time::now() - 1s").await;
    h.engine.tick_pass().await.unwrap();
    h
}

async fn workflow_run_id(h: &Harness) -> ids::Ref {
    let raw = h
        .one_string("SELECT VALUE type::string(id) FROM ONLY _00_workflow_run WHERE true LIMIT 1")
        .await
        .expect("a workflow run exists");
    ids::Ref::parse(&raw).expect("a well-formed record id")
}

#[tokio::test]
async fn a_workflow_dispatches_its_roots_in_parallel_and_blocks_the_rest() {
    let h = workflow_harness(diamond_workflow()).await;

    assert_eq!(h.step_status("extract-orders").await.as_deref(), Some("dispatched"));
    assert_eq!(h.step_status("extract-users").await.as_deref(), Some("dispatched"));
    assert_eq!(h.step_status("transform").await.as_deref(), Some("blocked"));
    assert_eq!(h.job_ids().await.len(), 2, "only the roots have jobs");
    assert_eq!(h.workflow_status().await.as_deref(), Some("running"));
}

#[tokio::test]
async fn a_join_waits_for_every_dependency_then_passes_their_outputs_on() {
    let h = workflow_harness(diamond_workflow()).await;

    // First branch done: the join must still wait.
    h.finish_job(
        &h.step_job("extract-orders").await.unwrap(),
        "success",
        json!({"fileId": "orders-1"}),
    )
    .await;
    h.engine.tick_pass().await.unwrap();
    assert_eq!(h.step_status("extract-orders").await.as_deref(), Some("success"));
    assert_eq!(h.step_status("transform").await.as_deref(), Some("blocked"), "join not satisfied");

    // Second branch done: the join dispatches, carrying both outputs.
    h.finish_job(
        &h.step_job("extract-users").await.unwrap(),
        "success",
        json!({"fileId": "users-1"}),
    )
    .await;
    h.engine.tick_pass().await.unwrap();
    assert_eq!(h.step_status("transform").await.as_deref(), Some("dispatched"));

    let transform_job = h.step_job("transform").await.unwrap();
    let orders: Option<String> = h
        .raw
        .query("SELECT VALUE payload.steps.`extract-orders`.fileId FROM ONLY type::record($id)")
        .bind(("id", transform_job.clone()))
        .await
        .expect("query")
        .take(0)
        .expect("take");
    assert_eq!(orders.as_deref(), Some("orders-1"), "a step reads its dependency's output");

    // The join's own completion fans out to both dependents.
    h.finish_job(&transform_job, "success", json!({"report": "r-1"})).await;
    h.engine.tick_pass().await.unwrap();
    assert_eq!(h.step_status("notify").await.as_deref(), Some("dispatched"));
    assert_eq!(h.step_status("archive").await.as_deref(), Some("dispatched"));
}

#[tokio::test]
async fn a_workflow_succeeds_only_once_every_step_has() {
    let h = workflow_harness(diamond_workflow()).await;
    for step in ["extract-orders", "extract-users"] {
        h.finish_job(&h.step_job(step).await.unwrap(), "success", json!({"fileId": step})).await;
    }
    h.engine.tick_pass().await.unwrap();
    h.finish_job(&h.step_job("transform").await.unwrap(), "success", json!({})).await;
    h.engine.tick_pass().await.unwrap();
    assert_eq!(h.workflow_status().await.as_deref(), Some("running"), "leaves still in flight");

    for step in ["notify", "archive"] {
        h.finish_job(&h.step_job(step).await.unwrap(), "success", json!({})).await;
    }
    h.engine.tick_pass().await.unwrap();

    assert_eq!(h.workflow_status().await.as_deref(), Some("success"));
    assert_eq!(
        h.one_string("SELECT VALUE status FROM ONLY _00_schedule_run WHERE true LIMIT 1")
            .await
            .as_deref(),
        Some("success"),
        "the outcome mirrors onto the schedule run that spawned it"
    );
}

#[tokio::test]
async fn halt_skips_everything_downstream_of_a_failure() {
    let h = workflow_harness(diamond_workflow()).await;

    h.fail_job(&h.step_job("extract-orders").await.unwrap(), "boom").await;
    h.engine.tick_pass().await.unwrap();

    assert_eq!(h.step_status("extract-orders").await.as_deref(), Some("failed"));
    assert_eq!(h.step_status("transform").await.as_deref(), Some("skipped"));
    assert_eq!(h.step_status("notify").await.as_deref(), Some("skipped"));
    assert_eq!(
        h.step_status("extract-users").await.as_deref(),
        Some("dispatched"),
        "a sibling already in flight is left to finish rather than abandoned"
    );

    // Once the in-flight sibling lands, the run terminalizes as failed.
    h.finish_job(&h.step_job("extract-users").await.unwrap(), "success", json!({})).await;
    h.engine.tick_pass().await.unwrap();
    assert_eq!(h.workflow_status().await.as_deref(), Some("failed"));
    assert_eq!(
        h.one_string("SELECT VALUE error.step FROM ONLY _00_workflow_run WHERE true LIMIT 1")
            .await
            .as_deref(),
        Some("extract-orders"),
        "the failing step is named"
    );
}

#[tokio::test]
async fn continue_independent_only_skips_the_affected_branch() {
    // extract-orders → transform        (this branch dies)
    // independent → after-independent   (this one must still run)
    let h = workflow_harness(json!({
        "kind": "workflow",
        "every_ms": 3_600_000,
        "target_table": "job",
        "workflow": {
            "target_table": "job",
            "steps": [
                {"name": "extract-orders", "path": "/a"},
                {"name": "transform", "path": "/b", "depends_on": ["extract-orders"]},
                {"name": "independent", "path": "/c"},
                {"name": "after-independent", "path": "/d", "depends_on": ["independent"]},
            ],
            "on_failure": "continue-independent",
        },
    }))
    .await;

    h.fail_job(&h.step_job("extract-orders").await.unwrap(), "boom").await;
    h.finish_job(&h.step_job("independent").await.unwrap(), "success", json!({})).await;
    h.engine.tick_pass().await.unwrap();

    assert_eq!(h.step_status("transform").await.as_deref(), Some("skipped"), "doomed branch");
    assert_eq!(
        h.step_status("after-independent").await.as_deref(),
        Some("dispatched"),
        "an unrelated branch keeps going"
    );

    h.finish_job(&h.step_job("after-independent").await.unwrap(), "success", json!({})).await;
    h.engine.tick_pass().await.unwrap();
    assert_eq!(
        h.workflow_status().await.as_deref(),
        Some("failed"),
        "the run still failed overall"
    );
}

#[tokio::test]
async fn a_retrying_step_is_not_treated_as_failed() {
    let h = workflow_harness(diamond_workflow()).await;
    let job = h.step_job("extract-orders").await.unwrap();

    // The runner's retry path puts the row back to `pending` between attempts.
    h.raw
        .query("UPDATE type::record($id) SET status = 'pending', retries = 1")
        .bind(("id", job.clone()))
        .await
        .unwrap();
    h.engine.tick_pass().await.unwrap();

    assert_eq!(
        h.step_status("extract-orders").await.as_deref(),
        Some("dispatched"),
        "per-step retry is the job's own retry budget; the step waits it out"
    );
    assert_eq!(h.workflow_status().await.as_deref(), Some("running"));
}

#[tokio::test]
async fn killing_a_run_skips_what_hasnt_started_and_kills_what_has() {
    let h = workflow_harness(diamond_workflow()).await;
    let run_id = workflow_run_id(&h).await;
    let root_jobs = [
        h.step_job("extract-orders").await.unwrap(),
        h.step_job("extract-users").await.unwrap(),
    ];

    // The operator writes only the flag; the engine does the rest.
    h.set(&format!("UPDATE {run_id} SET kill_requested = true")).await;
    h.engine.tick_pass().await.unwrap();

    assert_eq!(h.workflow_status().await.as_deref(), Some("killed"));
    assert_eq!(h.step_status("transform").await.as_deref(), Some("skipped"));
    let killed = h.kill.killed.lock().unwrap().clone();
    for job in root_jobs {
        assert!(killed.contains(&job), "in-flight step job {job} should have been killed");
    }
    assert_eq!(
        h.one_string("SELECT VALUE status FROM ONLY _00_schedule_run WHERE true LIMIT 1")
            .await
            .as_deref(),
        Some("killed")
    );
}

#[tokio::test]
async fn advancing_twice_concurrently_dispatches_each_step_once() {
    let h = workflow_harness(diamond_workflow()).await;
    for step in ["extract-orders", "extract-users"] {
        h.finish_job(&h.step_job(step).await.unwrap(), "success", json!({})).await;
    }
    let run_id = workflow_run_id(&h).await;

    // An ingest event and a heal pass can genuinely land at the same moment.
    let rival = h.rival();
    let (a, b) = tokio::join!(h.engine.advance_workflow(&run_id), rival.advance_workflow(&run_id));
    a.unwrap();
    b.unwrap();

    assert_eq!(
        h.count("SELECT VALUE count() FROM job WHERE path = '/buildReport' GROUP ALL").await,
        1,
        "the blocked→ready CAS means only one pass dispatches the join"
    );
}

#[tokio::test]
async fn a_dag_with_a_cycle_fails_its_run_instead_of_wedging_the_sweep() {
    let h = harness().await;
    h.define_schedule(
        "cyclic",
        json!({
            "kind": "workflow",
            "every_ms": 3_600_000,
            "target_table": "job",
            "workflow": {
                "target_table": "job",
                "steps": [
                    {"name": "a", "path": "/a", "depends_on": ["b"]},
                    {"name": "b", "path": "/b", "depends_on": ["a"]},
                ],
            },
        }),
    )
    .await;
    h.engine.tick_pass().await.unwrap();
    h.set("UPDATE _00_schedule:cyclic SET next_fire_at = time::now() - 1s").await;

    let report = h.engine.tick_pass().await.unwrap();
    assert_eq!(report.errored, 1, "the bad definition is reported, not panicked on");
    assert!(h.job_ids().await.is_empty());

    // And a healthy schedule still fires in the same sweep.
    h.define_schedule("nightly", every_5m_job()).await;
    h.engine.tick_pass().await.unwrap();
    h.set("UPDATE _00_schedule:nightly SET next_fire_at = time::now() - 1s").await;
    let report = h.engine.tick_pass().await.unwrap();
    assert_eq!(report.spawned, 1);
}
