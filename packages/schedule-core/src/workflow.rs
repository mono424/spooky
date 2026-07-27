//! Workflow DAG execution.
//!
//! A workflow run is a set of step rows plus the frozen DAG that produced them.
//! There is no in-memory execution state and no long-lived coordinator task:
//! every advancement reads the step rows, decides what changed, and writes the
//! transitions back under guards. That makes advancement idempotent, so it is
//! safe to trigger from a lost-event heal pass, from a job-completion event, and
//! from two of them at once.
//!
//! ```text
//! spawn ─▶ step rows (all blocked)
//!            │
//!            ├─ advance: deps satisfied? ─▶ CAS blocked→ready ─▶ CREATE job ─▶ dispatched
//!            │                                (winner of the CAS dispatches)
//!            ├─ job terminal ─▶ finalize step (success + output │ failed)
//!            ├─ a step failed ─▶ fail run, skip what can never run
//!            └─ nothing left non-terminal ─▶ finalize run, finalize schedule run
//! ```

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};

use crate::dag::{StepStatus, WorkflowDag};
use crate::db::{first_row, rows};
use crate::engine::{
    bind_ref, build_job_content, build_payload, last_error, row_ref, ScheduleEngine, Trigger,
    ROLLUP_NAME_AD_HOC, SCOPE_SCHEDULE,
};
use crate::spec::{OnFailure, ScheduleSpec, StepDef, HISTORY_FAILURES_ONLY};
use crate::{ids, sql};

/// A step row as the engine reads it back.
struct StepRow {
    id: ids::Ref,
    step: String,
    status: StepStatus,
    job_id: Option<String>,
    output: Option<Value>,
}

impl ScheduleEngine {
    /// Spawn a workflow run for one fire (one fan-out item). `false` = this fire
    /// already produced a run.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn spawn_workflow(
        &self,
        schedule_id: &ids::Ref,
        spec: &ScheduleSpec,
        run_key: &str,
        key: &str,
        fire_at: DateTime<Utc>,
        trigger: Trigger,
        item: Option<Value>,
    ) -> anyhow::Result<bool> {
        let Some(def) = spec.workflow.as_ref() else {
            anyhow::bail!("workflow schedule has no `workflow` definition");
        };
        // Validate again here even though the CLI already did: a hand-edited row
        // should fail its own run, not wedge the sweep.
        let dag = WorkflowDag::validate(def)?;

        let wf_run = ids::workflow_run(run_key);

        // The schedule run is the idempotency gate for the whole fire, so it goes
        // first — before the workflow run and its steps exist.
        let created = self
            .create_schedule_run(
                schedule_id,
                spec,
                run_key,
                key,
                fire_at,
                trigger,
                "running",
                None,
                None,
            )
            .await?;
        if !created {
            return Ok(false);
        }

        let mut content = Map::new();
        content.insert("schedule_name".into(), json!(spec.name));
        content.insert("workflow_name".into(), json!(spec.name));
        // Frozen: redeploying the workflow can't change a run already in flight.
        content.insert("dag".into(), serde_json::to_value(def)?);
        // Same reasoning for the history mode: a successful run is discarded as it
        // finalizes, and at that point its owning schedule run may already be gone, so
        // the mode has to travel with the run rather than be read back.
        if let Some(mode) = spec.history_mode.as_deref() {
            content.insert("history_mode".into(), json!(mode));
        }
        content.insert("status".into(), json!("running"));
        if let Some(table) = spec.target_table.as_deref() {
            content.insert("target_table".into(), json!(table));
        }
        if let Some(row) = item {
            content.insert("input".into(), row);
        }
        let mut binds = bind_ref("", &wf_run).to_vec();
        binds.extend(bind_ref("schedule", schedule_id));
        binds.push(("content", Value::Object(content)));
        self.create_if_absent(sql::CREATE_WORKFLOW_RUN, &binds).await?;

        for step in dag.steps() {
            let mut content = Map::new();
            content.insert("step".into(), json!(step.name));
            content.insert("depends_on".into(), json!(step.depends_on));
            content.insert("status".into(), json!("blocked"));
            let mut binds = bind_ref("", &ids::step_run(run_key, &step.name)).to_vec();
            binds.extend(bind_ref("wf", &wf_run));
            binds.push(("content", Value::Object(content)));
            self.create_if_absent(sql::CREATE_STEP_RUN, &binds).await?;
        }

        let mut binds = bind_ref("", &ids::schedule_run(run_key)).to_vec();
        binds.extend(bind_ref("wf", &wf_run));
        self.db.query(sql::LINK_WORKFLOW_RUN, &binds).await?;

        // Roots become ready through the same readiness pass as everything else.
        self.advance_workflow(&wf_run).await?;
        Ok(true)
    }

    /// Drive one workflow run as far as it can currently go.
    ///
    /// Safe to call concurrently with itself: each transition is a guarded
    /// compare-and-swap, and dispatch only happens for steps whose
    /// `blocked → ready` promotion this call won.
    pub async fn advance_workflow(&self, wf_run: &ids::Ref) -> anyhow::Result<()> {
        let Some(run) = self.load_workflow_run(wf_run).await? else {
            return Ok(());
        };
        // Only `running` runs advance — this is what makes the pass a no-op for
        // runs that were killed, replaced, or already finished.
        if run.get("status").and_then(Value::as_str) != Some("running") {
            return Ok(());
        }
        if run.get("kill_requested").and_then(Value::as_bool) == Some(true) {
            return self.kill_workflow_run(wf_run, &run).await;
        }

        let dag = match self.frozen_dag(&run) {
            Ok(dag) => dag,
            Err(e) => {
                return self
                    .fail_workflow_run(wf_run, json!({ "code": "bad_dag", "reason": e.to_string() }))
                    .await;
            }
        };

        let mut steps = self.load_step_rows(wf_run).await?;

        // 1. Land the terminal state of any step whose job has finished. The
        //    engine reads job rows but never writes their status.
        for step in steps.iter_mut() {
            if step.status != StepStatus::Dispatched {
                continue;
            }
            let Some(job_id) = step.job_id.as_deref() else { continue };
            let Some(job) = self.load_job(job_id).await? else {
                self.finalize_step(
                    &step.id,
                    "failed",
                    None,
                    Some(json!({ "code": "job_missing", "reason": "job row no longer exists" })),
                )
                .await?;
                step.status = StepStatus::Failed;
                continue;
            };
            match job.get("status").and_then(Value::as_str) {
                Some("success") => {
                    let output = job.get("result").cloned();
                    self.finalize_step(&step.id, "success", output.clone(), None).await?;
                    step.status = StepStatus::Success;
                    step.output = output;
                }
                Some("failed") => {
                    self.finalize_step(&step.id, "failed", None, last_error(&job)).await?;
                    step.status = StepStatus::Failed;
                }
                // pending / processing: the job is retrying or still running. A
                // step is only `failed` once its job row is terminally failed, so
                // per-step retry is just the job's own retry budget.
                _ => {}
            }
        }

        let statuses: BTreeMap<String, StepStatus> =
            steps.iter().map(|s| (s.step.clone(), s.status)).collect();
        let any_failed = steps.iter().any(|s| s.status == StepStatus::Failed);

        // 2. Propagate failure. `halt` stops everything that hasn't started;
        //    `continue-independent` skips only the branch below the failure and
        //    lets unrelated branches finish.
        if any_failed {
            let on_failure = self.on_failure(&run);
            let to_skip: Vec<ids::Ref> = match on_failure {
                OnFailure::Halt => steps
                    .iter()
                    .filter(|s| matches!(s.status, StepStatus::Blocked | StepStatus::Ready))
                    .map(|s| s.id.clone())
                    .collect(),
                OnFailure::ContinueIndependent => {
                    let doomed = dag.doomed_steps(&statuses);
                    let doomed_names: Vec<&str> = doomed.iter().map(|s| s.name.as_str()).collect();
                    steps
                        .iter()
                        .filter(|s| doomed_names.contains(&s.step.as_str()))
                        .map(|s| s.id.clone())
                        .collect()
                }
            };
            for step_ref in &to_skip {
                self.db.query(sql::SKIP_STEP, &bind_ref("", step_ref)).await?;
            }
            for step in steps.iter_mut() {
                if to_skip.contains(&step.id) {
                    step.status = StepStatus::Skipped;
                }
            }
        }

        // 3. Dispatch whatever is ready now. Recompute statuses: step 2 may have
        //    skipped some of them.
        if !matches!(self.on_failure(&run), OnFailure::Halt) || !any_failed {
            let statuses: BTreeMap<String, StepStatus> =
                steps.iter().map(|s| (s.step.clone(), s.status)).collect();
            let outputs: BTreeMap<String, Value> = steps
                .iter()
                .filter(|s| s.status == StepStatus::Success)
                .filter_map(|s| s.output.clone().map(|o| (s.step.clone(), o)))
                .collect();

            let ready: Vec<&StepDef> = dag.ready_steps(&statuses).collect();
            for step_def in ready {
                let Some(row) = steps.iter().find(|s| s.step == step_def.name) else { continue };
                // Winning this CAS is the right to dispatch. A concurrent pass
                // that loses it skips the step and never creates a second job.
                let claimed = self.db.query(sql::CLAIM_STEP_READY, &bind_ref("", &row.id)).await?;
                if first_row(claimed).is_none() {
                    continue;
                }
                self.dispatch_step(wf_run, &run, step_def, &row.id, &outputs).await?;
            }
        }

        // 4. Finalize when nothing can move any more.
        //
        // `!steps.is_empty()` is load-bearing: `Iterator::all` is `true` for an EMPTY
        // list and `find(Failed)` is then `None`, so a run whose step rows had vanished
        // would be reported a clean `success` — announcing work that provably never
        // finished. A run always has at least one step (an empty DAG is rejected at
        // validation), so no rows means something deleted them, not that there was
        // nothing to do.
        let steps = self.load_step_rows(wf_run).await?;
        let all_terminal = !steps.is_empty() && steps.iter().all(|s| s.status.is_terminal());
        if all_terminal {
            match steps.iter().find(|s| s.status == StepStatus::Failed) {
                Some(step) => {
                    self.fail_workflow_run(
                        wf_run,
                        json!({ "code": "step_failed", "step": step.step }),
                    )
                    .await?
                }
                None => self.complete_workflow_run(wf_run).await?,
            }
        }
        Ok(())
    }

    /// Create the step's job row and mark the step dispatched.
    async fn dispatch_step(
        &self,
        wf_run: &ids::Ref,
        run: &Value,
        step: &StepDef,
        step_row: &ids::Ref,
        outputs: &BTreeMap<String, Value>,
    ) -> anyhow::Result<()> {
        let run_key = &wf_run.key;
        let table = step
            .table
            .as_deref()
            .or_else(|| run.get("target_table").and_then(Value::as_str))
            .map(str::to_string);
        let Some(table) = table else {
            // The step is still `ready` here, so this needs the ready-guarded
            // statement rather than the dispatched-guarded finalize.
            let mut binds = bind_ref("", step_row).to_vec();
            binds.push((
                "error",
                json!({
                    "code": "no_target_table",
                    "reason": "step names no outbox table and the schedule has no target_table",
                }),
            ));
            self.db.query(sql::FAIL_UNDISPATCHABLE_STEP, &binds).await?;
            return Ok(());
        };

        let job = ids::step_job(&table, run_key, &step.name);

        // Only the direct dependencies' outputs are injected, so a long chain
        // can't grow an ever-larger payload.
        let deps: BTreeMap<String, Value> = step
            .depends_on
            .iter()
            .filter_map(|dep| outputs.get(dep).map(|out| (dep.clone(), out.clone())))
            .collect();
        let mut payload = build_payload(step.payload.as_ref(), None, Some(&deps));
        if let Some(input) = run.get("input") {
            if let Value::Object(map) = &mut payload {
                map.insert("input".into(), input.clone());
            }
        }

        let content = build_job_content(
            &step.path,
            payload,
            step.max_retries,
            step.retry_strategy.as_deref(),
            step.timeout,
        );

        // Deterministic job id: if the process died between this CREATE and the
        // MARK below, the retry collides here, and the step is stamped with the
        // id it would have used anyway.
        let mut binds = bind_ref("", &job).to_vec();
        binds.push(("content", content));
        self.create_if_absent(sql::CREATE_JOB, &binds).await?;

        let mut binds = bind_ref("", step_row).to_vec();
        binds.push(("job_id", json!(job.as_string())));
        self.db.query(sql::MARK_STEP_DISPATCHED, &binds).await?;
        Ok(())
    }

    /// Kill a run an operator asked to stop: skip what hasn't started, kill the
    /// jobs of everything in flight, then terminalize.
    async fn kill_workflow_run(&self, wf_run: &ids::Ref, run: &Value) -> anyhow::Result<()> {
        let steps = self.load_step_rows(wf_run).await?;
        for step in &steps {
            match step.status {
                StepStatus::Blocked | StepStatus::Ready => {
                    self.db.query(sql::SKIP_STEP, &bind_ref("", &step.id)).await?;
                }
                StepStatus::Dispatched => {
                    if let Some(job_id) = step.job_id.as_deref() {
                        if let Err(e) = self.kill.kill(job_id).await {
                            tracing::warn!(job_id, error = %e, "could not kill workflow step job");
                        }
                    }
                }
                _ => {}
            }
        }
        let mut patch = Map::new();
        patch.insert("status".into(), json!("killed"));
        patch.insert("error".into(), json!({ "code": "killed", "reason": "killed by operator" }));
        let mut binds = bind_ref("", wf_run).to_vec();
        binds.push(("patch", Value::Object(patch)));
        self.db.query(sql::FINALIZE_WORKFLOW_RUN, &binds).await?;
        self.finalize_owning_schedule_run(wf_run, run, "killed", None).await
    }

    /// Flag a run for the engine to kill on its next pass. Used by
    /// `concurrency: replace` and by `spky workflows kill`.
    pub(crate) async fn request_workflow_kill(&self, wf_run: &ids::Ref) -> anyhow::Result<()> {
        self.db.query(sql::REQUEST_WORKFLOW_KILL, &bind_ref("", wf_run)).await?;
        // Act immediately when we're already the engine; the flag is the durable
        // fallback if this pass dies mid-way.
        if let Some(run) = self.load_workflow_run(wf_run).await? {
            if run.get("status").and_then(Value::as_str) == Some("running") {
                self.kill_workflow_run(wf_run, &run).await?;
            }
        }
        Ok(())
    }

    /// Re-examine every `running` workflow run, and honour any pending kill.
    /// This is the polling fallback for a lost job-completion event.
    pub(crate) async fn heal_workflow_runs(&self) -> anyhow::Result<usize> {
        let mut healed = 0;

        for id in rows(self.db.query(sql::SELECT_KILL_REQUESTED, &[]).await?) {
            let Some(wf_run) = id.as_str().and_then(ids::Ref::parse) else { continue };
            if let Some(run) = self.load_workflow_run(&wf_run).await? {
                self.kill_workflow_run(&wf_run, &run).await?;
                healed += 1;
            }
        }

        for id in rows(self.db.query(sql::SELECT_RUNNING_WORKFLOW_RUNS, &[]).await?) {
            let Some(wf_run) = id.as_str().and_then(ids::Ref::parse) else { continue };
            self.advance_workflow(&wf_run).await?;
            healed += 1;
        }
        Ok(healed)
    }

    // -- small helpers ------------------------------------------------------

    async fn load_workflow_run(&self, wf_run: &ids::Ref) -> anyhow::Result<Option<Value>> {
        let results = self.db.query(sql::SELECT_WORKFLOW_RUN, &bind_ref("", wf_run)).await?;
        Ok(match results.into_iter().next() {
            Some(v @ Value::Object(_)) => Some(v),
            _ => None,
        })
    }

    async fn load_step_rows(&self, wf_run: &ids::Ref) -> anyhow::Result<Vec<StepRow>> {
        let raw = rows(self.db.query(sql::SELECT_STEP_RUNS, &bind_ref("wf", wf_run)).await?);
        Ok(raw
            .into_iter()
            .filter_map(|row| {
                Some(StepRow {
                    id: row_ref(&row)?,
                    step: row.get("step").and_then(Value::as_str)?.to_string(),
                    status: row
                        .get("status")
                        .and_then(Value::as_str)
                        .and_then(StepStatus::parse)?,
                    job_id: row.get("job_id").and_then(Value::as_str).map(str::to_string),
                    output: row.get("output").cloned().filter(|v| !v.is_null()),
                })
            })
            .collect())
    }

    fn frozen_dag(&self, run: &Value) -> anyhow::Result<WorkflowDag> {
        let dag = run.get("dag").ok_or_else(|| anyhow::anyhow!("workflow run has no dag"))?;
        let def = serde_json::from_value(dag.clone())?;
        Ok(WorkflowDag::validate(&def)?)
    }

    fn on_failure(&self, run: &Value) -> OnFailure {
        run.get("dag")
            .and_then(|dag| dag.get("on_failure"))
            .and_then(Value::as_str)
            .and_then(|s| match s {
                "continue-independent" => Some(OnFailure::ContinueIndependent),
                "halt" => Some(OnFailure::Halt),
                _ => None,
            })
            .unwrap_or_default()
    }

    async fn finalize_step(
        &self,
        step_row: &ids::Ref,
        status: &str,
        output: Option<Value>,
        error: Option<Value>,
    ) -> anyhow::Result<()> {
        let mut patch = Map::new();
        patch.insert("status".into(), json!(status));
        if let Some(output) = output.filter(|v| !v.is_null()) {
            patch.insert("output".into(), output);
        }
        if let Some(error) = error {
            patch.insert("error".into(), error);
        }
        let mut binds = bind_ref("", step_row).to_vec();
        binds.push(("patch", Value::Object(patch)));
        self.db.query(sql::FINALIZE_STEP, &binds).await?;
        Ok(())
    }

    async fn complete_workflow_run(&self, wf_run: &ids::Ref) -> anyhow::Result<()> {
        let mut patch = Map::new();
        patch.insert("status".into(), json!("success"));
        let mut binds = bind_ref("", wf_run).to_vec();
        binds.push(("patch", Value::Object(patch)));
        // `RETURN AFTER` rather than a re-read: by the time this returns the row may
        // be on its way out, and reading it back to find `schedule_name` is what
        // would strand the owning schedule run as `running` forever.
        let finalized = self.db.query(sql::FINALIZE_WORKFLOW_RUN, &binds).await?;
        let Some(run) = first_row(finalized) else { return Ok(()) };
        self.finalize_owning_schedule_run(wf_run, &run, "success", None).await?;
        // Strictly last: everything above has already taken what it needs off the row.
        self.discard_workflow_if_failures_only(wf_run, &run).await;
        Ok(())
    }

    async fn fail_workflow_run(&self, wf_run: &ids::Ref, error: Value) -> anyhow::Result<()> {
        let mut patch = Map::new();
        patch.insert("status".into(), json!("failed"));
        patch.insert("error".into(), error.clone());
        let mut binds = bind_ref("", wf_run).to_vec();
        binds.push(("patch", Value::Object(patch)));
        // Same shape as the success path. A failed run is never discarded — its rows are
        // the whole reason `failures-only` exists.
        let finalized = self.db.query(sql::FINALIZE_WORKFLOW_RUN, &binds).await?;
        if let Some(run) = first_row(finalized) {
            self.finalize_owning_schedule_run(wf_run, &run, "failed", Some(error)).await?;
        }
        Ok(())
    }

    /// Under `history: failures-only`, a workflow run that succeeded leaves nothing
    /// behind: its steps, its step jobs and the run row itself all go.
    ///
    /// Called ONLY from the success path. `kill_workflow_run` and the `bad_dag` failure
    /// finalize a run while steps are still `dispatched`/`blocked`, so "the run
    /// finalized" does not imply "its steps are terminal" — only `success` does, because
    /// it is gated on `all_terminal`.
    ///
    /// Everything that reads the run has already run by this point: the outcome is
    /// mirrored onto the owning schedule run, and the rollup is folded here BEFORE the
    /// delete, because afterwards there is nothing left to count.
    async fn discard_workflow_if_failures_only(&self, wf_run: &ids::Ref, run: &Value) {
        if run.get("history_mode").and_then(Value::as_str) != Some(HISTORY_FAILURES_ONLY) {
            return;
        }

        // `_00_workflow_run` has no `fire_at`, so the hourly bucket comes off
        // `created_at`. An ad-hoc `spky workflows trigger` run has no owning schedule;
        // count it under the same placeholder the prune's fold uses rather than dropping
        // it on the floor.
        let name =
            run.get("schedule_name").and_then(Value::as_str).unwrap_or(ROLLUP_NAME_AD_HOC);
        let at = run
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(crate::cron::parse_datetime)
            .unwrap_or_else(Utc::now);
        self.fold_rollup_one(run, at, SCOPE_SCHEDULE, "success", name).await;

        // The step rows carry the job ids, so they have to be read before they go. Their
        // `output` was copied onto them when each step finalized, so nothing downstream
        // needs the job rows any more.
        let steps = rows(
            self.db
                .query(sql::SELECT_STEP_RUNS, &bind_ref("wf", wf_run))
                .await
                .unwrap_or_default(),
        );
        let jobs: Vec<String> = steps
            .iter()
            .filter_map(|s| s.get("job_id").and_then(Value::as_str))
            .filter_map(ids::Ref::parse)
            .filter(|j| {
                sql::is_plain_identifier(&j.table) && sql::is_plain_identifier(&j.key)
            })
            .map(|j| j.as_string())
            .collect();
        if !jobs.is_empty() {
            if let Err(e) = self.db.query(&sql::delete_records(&jobs), &[]).await {
                tracing::warn!(error = %e, "could not discard a workflow's job rows");
            }
        }

        // Steps and run together, steps first — see DELETE_WORKFLOW_RUN_CASCADE.
        if let Err(e) =
            self.db.query(sql::DELETE_WORKFLOW_RUN_CASCADE, &bind_ref("", wf_run)).await
        {
            // Leaving the rows behind is harmless: retention collects them later.
            tracing::warn!(run = %wf_run, error = %e, "could not discard a successful workflow run");
        }
    }

    /// Mirror a workflow run's outcome onto the schedule run that spawned it.
    /// The link is derived from the shared run key rather than read back, so a
    /// crash before `LINK_WORKFLOW_RUN` can't orphan the history row.
    async fn finalize_owning_schedule_run(
        &self,
        wf_run: &ids::Ref,
        run: &Value,
        status: &str,
        error: Option<Value>,
    ) -> anyhow::Result<()> {
        // Ad-hoc runs (`spky workflows trigger`) have no owning schedule.
        if run.get("schedule_name").and_then(Value::as_str).is_none() {
            return Ok(());
        }
        // The run key is shared, so the sibling history row is derivable — no
        // need to read the link back, which a crash could have left unwritten.
        self.finalize_schedule_run(&ids::schedule_run(&wf_run.key), status, error).await
    }

    /// `CREATE` that treats a duplicate id as success — the deterministic-id
    /// idempotency check, in the one place every creator can share.
    pub(crate) async fn create_if_absent(
        &self,
        stmt: &str,
        binds: &[(&str, Value)],
    ) -> anyhow::Result<bool> {
        match self.db.query(stmt, binds).await {
            Ok(_) => Ok(true),
            Err(e) if e.is_already_exists() => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
}
