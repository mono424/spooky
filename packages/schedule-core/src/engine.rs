//! The engine: one `tick_pass` per sweep, plus the healing that makes event
//! delivery best-effort instead of load-bearing.
//!
//! ```text
//! tick_pass
//!   ├─ plan_pass      compute next_fire_at for unplanned schedules
//!   ├─ fire_pass      due + operator-triggered schedules → claim → fan out → spawn
//!   ├─ heal_pass      reconcile runs whose job already finished (lost event)
//!   └─ prune_pass     drop terminal history past the retention window
//! ```
//!
//! Spawning creates a `pending` row in the app's outbox table and stops there:
//! the existing ingest → pickup → `JobRunner` path executes it, so every
//! guarantee that machinery already has (claim/lease, retry backoff, kill,
//! recovery sweeps) applies to scheduled work for free. The engine's own writes
//! are confined to the `_00_schedule*` tables.

use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Duration, SecondsFormat, Utc};
use serde_json::{json, Map, Value};

use crate::db::{first_row, rows, ScheduleDb, ScheduleDbError};
use crate::kill::JobKill;
use crate::spec::{Concurrency, ScheduleKind, ScheduleSpec};
use crate::{ids, sql};

/// Tunables. Defaults are deliberate rather than configurable-by-accident: a
/// host may override them, but nothing reads the environment here (the portable
/// core never does).
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Upper bound on rows a single `forEach` fire may fan out over. A runaway
    /// query should degrade to a logged error, not spawn a hundred thousand jobs.
    pub fanout_max: usize,
    /// How long finished history is kept.
    pub history_max_age: Duration,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self { fanout_max: 10_000, history_max_age: Duration::days(30) }
    }
}

/// What one `tick_pass` did. Returned for logging and asserted on in tests.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    pub planned: usize,
    pub fired: usize,
    pub spawned: usize,
    pub skipped: usize,
    pub replaced: usize,
    pub healed: usize,
    pub pruned: usize,
    /// Schedules whose plan or fan-out failed. Recorded on the row's
    /// `last_error`; one bad schedule never aborts the sweep.
    pub errored: usize,
}

pub struct ScheduleEngine {
    pub(crate) db: Arc<dyn ScheduleDb>,
    pub(crate) kill: Arc<dyn JobKill>,
    cfg: EngineConfig,
}

/// How a fire was initiated — recorded on the run row so `spky schedules runs`
/// can distinguish a cron fire from someone hitting `trigger`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Trigger {
    Cron,
    Manual,
}

impl Trigger {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Trigger::Cron => "cron",
            Trigger::Manual => "manual",
        }
    }
}

impl ScheduleEngine {
    pub fn new(db: Arc<dyn ScheduleDb>, kill: Arc<dyn JobKill>, cfg: EngineConfig) -> Self {
        Self { db, kill, cfg }
    }

    pub fn db(&self) -> &Arc<dyn ScheduleDb> {
        &self.db
    }

    /// One sweep. Never returns `Err` for a single bad schedule — those land on
    /// the schedule's `last_error` and are counted in the report; only a
    /// transport-level failure reading the schedule table propagates.
    pub async fn tick_pass(&self) -> anyhow::Result<TickReport> {
        let now = Utc::now();
        let mut report = TickReport::default();

        self.plan_pass(now, &mut report).await?;
        self.fire_pass(now, &mut report).await?;
        self.heal_pass(&mut report).await?;
        self.prune_pass(now, &mut report).await?;

        Ok(report)
    }

    // -- planning -----------------------------------------------------------

    /// Give every unplanned schedule a `next_fire_at`. Runs before firing so a
    /// freshly deployed schedule can fire in the same sweep it was planned in.
    async fn plan_pass(&self, now: DateTime<Utc>, report: &mut TickReport) -> anyhow::Result<()> {
        let unplanned = rows(self.db.query(sql::SELECT_UNPLANNED, &[]).await?);
        for row in unplanned {
            let Some(id) = row_ref(&row) else { continue };
            let spec = match ScheduleSpec::from_row(&row) {
                Ok(spec) => spec,
                Err(e) => {
                    self.record_error(&id, &format!("unreadable schedule row: {e}"), report).await;
                    continue;
                }
            };
            let next = match spec.fire_spec().and_then(|f| f.next_fire_after(now)) {
                Ok(next) => next,
                Err(e) => {
                    self.record_error(&id, &e.to_string(), report).await;
                    continue;
                }
            };
            let mut binds = bind_ref("", &id).to_vec();
            binds.push(("next", json!(stamp(next))));
            match self.db.query(sql::PLAN_NEXT_FIRE, &binds).await {
                Ok(_) => report.planned += 1,
                Err(e) => {
                    tracing::warn!(schedule = %id, error = %e, "could not store next fire time");
                }
            }
        }
        Ok(())
    }

    // -- firing -------------------------------------------------------------

    async fn fire_pass(&self, now: DateTime<Utc>, report: &mut TickReport) -> anyhow::Result<()> {
        // Cron fires: CAS on the fire time we observed, and advance the clock to
        // the next occurrence after NOW — not after the missed slot. Downtime
        // therefore coalesces into a single catch-up fire instead of replaying
        // every slot it slept through.
        for row in rows(self.db.query(sql::SELECT_DUE, &[]).await?) {
            let Some((id, spec)) = self.read_spec(&row, report).await else { continue };
            let Some(observed) = row.get("next_fire_at").and_then(Value::as_str) else { continue };

            let next = match spec.fire_spec().and_then(|f| f.next_fire_after(now)) {
                Ok(next) => next,
                Err(e) => {
                    self.record_error(&id, &e.to_string(), report).await;
                    continue;
                }
            };

            let mut binds = bind_ref("", &id).to_vec();
            binds.extend([
                ("fire", json!(stamp(now))),
                ("next", json!(stamp(next))),
                ("observed", json!(observed)),
            ]);
            let claimed = self.db.query(sql::CLAIM_FIRE, &binds).await?;
            if first_row(claimed).is_none() {
                tracing::debug!(schedule = %id, "fire already claimed elsewhere");
                continue;
            }
            report.fired += 1;
            self.fan_out(&id, &spec, now, Trigger::Cron, report).await;
        }

        // Operator triggers: same spawn path, but the cron clock is untouched so
        // a manual run doesn't shift the schedule.
        for row in rows(self.db.query(sql::SELECT_TRIGGERED, &[]).await?) {
            let Some((id, spec)) = self.read_spec(&row, report).await else { continue };
            let Some(observed) = row.get("trigger_requested_at").and_then(Value::as_str) else {
                continue;
            };

            let mut binds = bind_ref("", &id).to_vec();
            binds.extend([("fire", json!(stamp(now))), ("observed", json!(observed))]);
            let claimed = self.db.query(sql::CLAIM_TRIGGER, &binds).await?;
            if first_row(claimed).is_none() {
                continue;
            }
            report.fired += 1;
            self.fan_out(&id, &spec, now, Trigger::Manual, report).await;
        }

        Ok(())
    }

    /// Resolve the fan-out rows for a fire and spawn one run per row.
    async fn fan_out(
        &self,
        id: &ids::Ref,
        spec: &ScheduleSpec,
        fire_at: DateTime<Utc>,
        trigger: Trigger,
        report: &mut TickReport,
    ) {
        let items = match self.fan_out_rows(spec).await {
            Ok(items) => items,
            Err(e) => {
                self.record_error(id, &format!("forEach query failed: {e}"), report).await;
                return;
            }
        };

        for item in items {
            let key = item.as_ref().map(|row| fan_out_key(row, spec.key_field())).unwrap_or_default();
            if let Err(e) = self.spawn_one(id, spec, fire_at, trigger, &key, item, report).await {
                // A single item must never take down the rest of the fan-out.
                tracing::warn!(schedule = %id, key = %key, error = %e, "could not spawn run");
                report.errored += 1;
            }
        }
    }

    /// `None` = no fan-out (one run per fire); `Some(rows)` = one run per row.
    async fn fan_out_rows(&self, spec: &ScheduleSpec) -> Result<Vec<Option<Value>>, ScheduleDbError> {
        let Some(query) = spec.for_each.as_deref() else {
            return Ok(vec![None]);
        };
        let mut items = rows(self.db.query(query, &[]).await?);
        if items.len() > self.cfg.fanout_max {
            // Truncation is logged loudly: silently covering "most" of the fan-out
            // reads as success when it isn't.
            tracing::error!(
                schedule = %spec.name,
                rows = items.len(),
                limit = self.cfg.fanout_max,
                "forEach returned more rows than the fan-out limit — dropping the excess"
            );
            items.truncate(self.cfg.fanout_max);
        }
        Ok(items.into_iter().map(Some).collect())
    }

    /// Apply the concurrency policy, then spawn (or record a skip).
    async fn spawn_one(
        &self,
        schedule_id: &ids::Ref,
        spec: &ScheduleSpec,
        fire_at: DateTime<Utc>,
        trigger: Trigger,
        key: &str,
        item: Option<Value>,
        report: &mut TickReport,
    ) -> anyhow::Result<()> {
        let run_key = ids::run_key(&spec.name, fire_at.timestamp_millis(), key);
        let run_ref = ids::schedule_run(&run_key);

        match spec.concurrency {
            Concurrency::Allow => {}
            Concurrency::Skip => {
                if self.active_run_count(&spec.name, key).await? > 0 {
                    // Recorded rather than dropped: an operator needs to see that
                    // ticks are being suppressed, not just that nothing ran.
                    self.create_schedule_run(
                        schedule_id,
                        spec,
                        &run_key,
                        key,
                        fire_at,
                        trigger,
                        item,
                        "skipped",
                        None,
                        None,
                    )
                    .await?;
                    report.skipped += 1;
                    return Ok(());
                }
            }
            Concurrency::Replace => {
                let replaced = self.replace_active_runs(&spec.name, key).await?;
                report.replaced += replaced;
            }
        }

        match spec.kind {
            ScheduleKind::Job => {
                let Some(table) = spec.target_table.as_deref() else {
                    anyhow::bail!("schedule has no target_table");
                };
                let Some(path) = spec.path.as_deref() else {
                    anyhow::bail!("schedule has no path");
                };
                let job = ids::job(table, &run_key);

                // Run row first: if the job CREATE lands and the process dies
                // before the run row exists, nothing would ever observe the job.
                let created = self
                    .create_schedule_run(
                        schedule_id,
                        spec,
                        &run_key,
                        key,
                        fire_at,
                        trigger,
                        item.clone(),
                        "running",
                        Some(&job.as_string()),
                        None,
                    )
                    .await?;
                if !created {
                    tracing::debug!(schedule = %schedule_id, key, "fire already spawned");
                    return Ok(());
                }

                let payload = build_payload(spec.payload.as_ref(), item.as_ref(), None);
                let content = build_job_content(
                    path,
                    payload,
                    spec.max_retries,
                    spec.retry_strategy.as_deref(),
                    spec.timeout,
                );
                let mut binds = bind_ref("", &job).to_vec();
                binds.push(("content", content));
                match self.db.query(sql::CREATE_JOB, &binds).await {
                    Ok(_) => report.spawned += 1,
                    Err(e) if e.is_already_exists() => {
                        tracing::debug!(job = %job, "job row already exists");
                    }
                    Err(e) => {
                        // The run row exists but its job doesn't and never will;
                        // fail it now rather than leaving it `running` forever.
                        self.finalize_schedule_run(
                            &run_ref,
                            "failed",
                            Some(json!({ "code": "spawn_failed", "reason": e.to_string() })),
                        )
                        .await?;
                        return Err(e.into());
                    }
                }
            }
            ScheduleKind::Workflow => {
                let spawned = self
                    .spawn_workflow(schedule_id, spec, &run_key, key, fire_at, trigger, item)
                    .await?;
                if spawned {
                    report.spawned += 1;
                }
            }
        }
        Ok(())
    }

    async fn active_run_count(&self, name: &str, key: &str) -> Result<i64, ScheduleDbError> {
        let results =
            self.db.query(sql::COUNT_ACTIVE_RUNS, &[("name", json!(name)), ("key", json!(key))]).await?;
        Ok(first_row(results).and_then(|v| v.as_i64()).unwrap_or(0))
    }

    /// Kill whatever is in flight for this key and mark those runs `replaced`.
    /// Best-effort by nature (the request may already be mid-flight), which is
    /// why the finalize is guarded — a late completion can't relabel the run.
    async fn replace_active_runs(&self, name: &str, key: &str) -> anyhow::Result<usize> {
        let active = rows(
            self.db
                .query(sql::SELECT_ACTIVE_RUNS, &[("name", json!(name)), ("key", json!(key))])
                .await?,
        );
        let mut count = 0;
        for run in active {
            let Some(run_ref) = row_ref(&run) else { continue };
            if let Some(job_id) = run.get("job_id").and_then(Value::as_str) {
                if let Err(e) = self.kill.kill(job_id).await {
                    tracing::warn!(job_id, error = %e, "could not kill replaced job");
                }
            }
            if let Some(wf_run) = run.get("workflow_run").and_then(Value::as_str) {
                if let Some(wf_ref) = ids::Ref::parse(wf_run) {
                    self.request_workflow_kill(&wf_ref).await?;
                }
            }
            self.finalize_schedule_run(&run_ref, "replaced", None).await?;
            count += 1;
        }
        Ok(count)
    }

    /// Create the history row. `false` = this fire already produced one.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn create_schedule_run(
        &self,
        schedule_id: &ids::Ref,
        spec: &ScheduleSpec,
        run_key: &str,
        key: &str,
        fire_at: DateTime<Utc>,
        trigger: Trigger,
        item: Option<Value>,
        status: &str,
        job_id: Option<&str>,
        workflow_run: Option<&str>,
    ) -> anyhow::Result<bool> {
        let mut content = Map::new();
        content.insert("schedule_name".into(), json!(spec.name));
        content.insert("key".into(), json!(key));
        content.insert("kind".into(), json!(match spec.kind {
            ScheduleKind::Job => "job",
            ScheduleKind::Workflow => "workflow",
        }));
        content.insert("status".into(), json!(status));
        content.insert("trigger".into(), json!(trigger.as_str()));
        // Absent optionals are OMITTED, never nulled: `option<T>` rejects NULL.
        if let Some(job_id) = job_id {
            content.insert("job_id".into(), json!(job_id));
        }
        if let Some(wf) = workflow_run {
            content.insert("workflow_run".into(), json!(wf));
        }
        if let Some(row) = item {
            content.insert("row".into(), row);
        }
        let mut binds = bind_ref("", &ids::schedule_run(run_key)).to_vec();
        binds.extend(bind_ref("schedule", schedule_id));
        binds.extend([("content", Value::Object(content)), ("fire", json!(stamp(fire_at)))]);
        // A run that is born terminal (a suppressed `skip`) needs its
        // `finished_at` too, and that has to be cast in the statement.
        let stmt = if status == "running" {
            sql::CREATE_SCHEDULE_RUN
        } else {
            sql::CREATE_TERMINAL_SCHEDULE_RUN
        };
        let result = self.db.query(stmt, &binds).await;

        match result {
            Ok(_) => Ok(true),
            // The deterministic id turns a duplicate into the idempotency check:
            // two tickers racing the same fire, or a replayed sweep, collide here
            // instead of double-spawning.
            Err(e) if e.is_already_exists() => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    pub(crate) async fn finalize_schedule_run(
        &self,
        run: &ids::Ref,
        status: &str,
        error: Option<Value>,
    ) -> anyhow::Result<()> {
        let mut patch = Map::new();
        patch.insert("status".into(), json!(status));
        if let Some(error) = error {
            patch.insert("error".into(), error);
        }
        let mut binds = bind_ref("", run).to_vec();
        binds.push(("patch", Value::Object(patch)));
        self.db.query(sql::FINALIZE_SCHEDULE_RUN, &binds).await?;
        Ok(())
    }

    // -- observing ----------------------------------------------------------

    /// React to a job that just reached a terminal status.
    ///
    /// Called from the ingest path: the runner's terminal `UPDATE` fires the
    /// table's `_00_<table>_mutation` event, which POSTs `/ingest`, and the host
    /// hands the job id here. That is the fast path — delivery is best-effort, so
    /// [`Self::heal_pass`] reaches the same conclusion within one sweep if the
    /// event is lost. Both are idempotent and guarded, so a duplicate is a no-op.
    ///
    /// Returns `true` when the job belonged to a schedule or a workflow step.
    pub async fn observe_job_terminal(&self, job_id: &str, status: &str) -> anyhow::Result<bool> {
        if !matches!(status, "success" | "failed") {
            return Ok(false);
        }

        // A one-shot scheduled job: mirror the outcome onto its run.
        let runs = rows(self.db.query(sql::FIND_RUN_BY_JOB, &[("job_id", json!(job_id))]).await?);
        let mut matched = false;
        for run in runs {
            let Some(run_ref) = row_ref(&run) else { continue };
            let error = match status {
                "failed" => self.load_job(job_id).await?.as_ref().and_then(last_error),
                _ => None,
            };
            self.finalize_schedule_run(&run_ref, status, error).await?;
            matched = true;
        }

        // A workflow step: let the DAG advance, which finalizes the step, may
        // dispatch its dependents, and may finish the run.
        let steps = rows(self.db.query(sql::FIND_STEP_BY_JOB, &[("job_id", json!(job_id))]).await?);
        for step in steps {
            let Some(wf_run) = step
                .get("workflow_run")
                .and_then(Value::as_str)
                .and_then(ids::Ref::parse)
            else {
                continue;
            };
            self.advance_workflow(&wf_run).await?;
            matched = true;
        }

        Ok(matched)
    }

    // -- healing ------------------------------------------------------------

    /// Reconcile state that an event should have delivered but didn't.
    ///
    /// The engine learns about finished jobs from the ingest event the runner's
    /// terminal UPDATE fires, and that delivery is best-effort — an SSP restart
    /// or a dropped `http::post` loses it. So every sweep also asks the database
    /// directly: any `running` job-run whose job row is already terminal gets
    /// finalized here, capping the worst-case delay at one sweep interval.
    async fn heal_pass(&self, report: &mut TickReport) -> anyhow::Result<()> {
        for run in rows(self.db.query(sql::SELECT_RUNNING_JOB_RUNS, &[]).await?) {
            let Some(run_ref) = row_ref(&run) else { continue };
            let Some(job_id) = run.get("job_id").and_then(Value::as_str) else { continue };
            let Some(job) = self.load_job(job_id).await? else {
                // The job row is gone (pruned, or an operator deleted it): the
                // run can never complete, so stop calling it `running`.
                self.finalize_schedule_run(
                    &run_ref,
                    "failed",
                    Some(json!({ "code": "job_missing", "reason": "job row no longer exists" })),
                )
                .await?;
                report.healed += 1;
                continue;
            };
            let status = job.get("status").and_then(Value::as_str).unwrap_or("");
            match status {
                "success" => {
                    self.finalize_schedule_run(&run_ref, "success", None).await?;
                    report.healed += 1;
                }
                "failed" => {
                    self.finalize_schedule_run(&run_ref, "failed", last_error(&job)).await?;
                    report.healed += 1;
                }
                _ => {}
            }
        }

        report.healed += self.heal_workflow_runs().await?;
        Ok(())
    }

    pub(crate) async fn load_job(&self, job_id: &str) -> anyhow::Result<Option<Value>> {
        let Some(job) = ids::Ref::parse(job_id) else { return Ok(None) };
        let results = self.db.query(sql::SELECT_JOB, &bind_ref("", &job)).await?;
        Ok(match results.into_iter().next() {
            Some(v @ Value::Object(_)) => Some(v),
            _ => None,
        })
    }

    // -- retention ----------------------------------------------------------

    async fn prune_pass(&self, now: DateTime<Utc>, report: &mut TickReport) -> anyhow::Result<()> {
        let before = json!(stamp(now - self.cfg.history_max_age));
        for stmt in [sql::PRUNE_STEP_RUNS, sql::PRUNE_WORKFLOW_RUNS, sql::PRUNE_SCHEDULE_RUNS] {
            match self.db.query(stmt, &[("before", before.clone())]).await {
                Ok(results) => report.pruned += rows(results).len(),
                Err(e) => tracing::warn!(error = %e, "history prune failed"),
            }
        }
        Ok(())
    }

    // -- shared helpers -----------------------------------------------------

    /// Parse a schedule row, recording (rather than propagating) a bad one.
    async fn read_spec(
        &self,
        row: &Value,
        report: &mut TickReport,
    ) -> Option<(ids::Ref, ScheduleSpec)> {
        let id = row_ref(row)?;
        match ScheduleSpec::from_row(row) {
            Ok(spec) => Some((id, spec)),
            Err(e) => {
                self.record_error(&id, &format!("unreadable schedule row: {e}"), report).await;
                None
            }
        }
    }

    async fn record_error(&self, id: &ids::Ref, error: &str, report: &mut TickReport) {
        report.errored += 1;
        tracing::warn!(schedule = %id, error, "schedule error");
        let mut binds = bind_ref("", id).to_vec();
        binds.push(("error", json!(error)));
        if let Err(e) = self.db.query(sql::RECORD_ERROR, &binds).await {
            tracing::warn!(schedule = %id, error = %e, "could not record schedule error");
        }
    }
}

/// Bind a record id the only way that is safe: as a (table, key) pair. `prefix`
/// names the pair — `""` for the statement's primary `$tb`/`$key`, or e.g.
/// `"schedule"` for a second reference in the same statement.
pub(crate) fn bind_ref(prefix: &str, r: &ids::Ref) -> [(&'static str, Value); 2] {
    match prefix {
        "schedule" => [("schedule_tb", json!(r.table)), ("schedule_key", json!(r.key))],
        "wf" => [("wf_tb", json!(r.table)), ("wf_key", json!(r.key))],
        _ => [("tb", json!(r.table)), ("key", json!(r.key))],
    }
}

/// RFC 3339 with millisecond precision — the form SurrealDB's flattened values
/// hand back, so a stamp we write can be compared against one we read.
pub(crate) fn stamp(at: DateTime<Utc>) -> String {
    at.to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// `id` out of a flattened row, split into the (table, key) pair every statement
/// needs. Flattened ids arrive backtick-quoted when the key needs escaping, which
/// [`ids::Ref::parse`] strips.
pub(crate) fn row_ref(row: &Value) -> Option<ids::Ref> {
    row.get("id").and_then(Value::as_str).and_then(ids::Ref::parse)
}

/// Last entry of a job row's `errors` array, for the run's `error` field.
pub(crate) fn last_error(job: &Value) -> Option<Value> {
    job.get("errors").and_then(Value::as_array).and_then(|errors| errors.last().cloned())
}

/// Concurrency key for a fan-out row: the value at `key_field`, stringified.
fn fan_out_key(row: &Value, key_field: &str) -> String {
    match row.get(key_field) {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        // No such field: every row shares one key, so `skip` serializes the
        // whole fan-out. Surprising, but safer than silently spawning overlap.
        None => String::new(),
    }
}

/// Payload handed to a spawned job.
///
/// There is no `{{…}}` template language: the engine injects what a job could
/// need under reserved keys and the backend reads them. `row` is the fan-out row,
/// `steps` maps each direct dependency's name to its captured output.
pub(crate) fn build_payload(
    base: Option<&Value>,
    row: Option<&Value>,
    step_outputs: Option<&BTreeMap<String, Value>>,
) -> Value {
    let mut payload = match base {
        Some(Value::Object(map)) => map.clone(),
        _ => Map::new(),
    };
    if let Some(row) = row {
        payload.insert("row".into(), row.clone());
    }
    if let Some(outputs) = step_outputs {
        if !outputs.is_empty() {
            let steps: Map<String, Value> =
                outputs.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            payload.insert("steps".into(), Value::Object(steps));
        }
    }
    Value::Object(payload)
}

/// The outbox row the engine creates. `status` is deliberately absent — the
/// schema defaults it to `pending`, and the runner owns it from there.
pub(crate) fn build_job_content(
    path: &str,
    payload: Value,
    max_retries: Option<i64>,
    retry_strategy: Option<&str>,
    timeout: Option<i64>,
) -> Value {
    let mut content = Map::new();
    content.insert("path".into(), json!(path));
    content.insert("payload".into(), payload);
    // Omit rather than null: the outbox schema's own DEFAULTs then apply.
    if let Some(max_retries) = max_retries {
        content.insert("max_retries".into(), json!(max_retries));
    }
    if let Some(strategy) = retry_strategy {
        content.insert("retry_strategy".into(), json!(strategy));
    }
    if let Some(timeout) = timeout {
        content.insert("timeout".into(), json!(timeout));
    }
    Value::Object(content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cron::parse_datetime;

    #[test]
    fn job_content_never_carries_status() {
        let content = build_job_content("/run", json!({}), Some(3), Some("linear"), None);
        let obj = content.as_object().unwrap();
        assert!(!obj.contains_key("status"), "the runner is the single status writer");
        assert!(!obj.contains_key("timeout"), "absent optionals are omitted, not nulled");
        assert_eq!(obj["max_retries"], json!(3));
    }

    #[test]
    fn payload_injects_row_and_step_outputs() {
        let mut outputs = BTreeMap::new();
        outputs.insert("extract".to_string(), json!({"fileId": "f_1"}));
        let payload = build_payload(
            Some(&json!({"olderThanDays": 30})),
            Some(&json!({"id": "connection:alice"})),
            Some(&outputs),
        );
        assert_eq!(payload["olderThanDays"], json!(30));
        assert_eq!(payload["row"]["id"], json!("connection:alice"));
        assert_eq!(payload["steps"]["extract"]["fileId"], json!("f_1"));
    }

    #[test]
    fn payload_omits_absent_sections() {
        let payload = build_payload(None, None, None);
        assert_eq!(payload, json!({}));
    }

    #[test]
    fn fan_out_key_stringifies_non_strings() {
        assert_eq!(fan_out_key(&json!({"id": "connection:a"}), "id"), "connection:a");
        assert_eq!(fan_out_key(&json!({"n": 7}), "n"), "7");
        assert_eq!(fan_out_key(&json!({"other": 1}), "id"), "");
    }

    #[test]
    fn stamps_round_trip_through_millis() {
        let now = Utc::now();
        let parsed = parse_datetime(&stamp(now)).unwrap();
        assert_eq!(parsed.timestamp_millis(), now.timestamp_millis());
    }
}
