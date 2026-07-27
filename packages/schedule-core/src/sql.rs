//! Every SurrealQL statement the engine runs, in one place.
//!
//! Kept as constants (rather than inlined at the call sites) for the same
//! reason `PENDING_DUE_CLAUSE` is shared by both job-recovery sweeps: the
//! singlenode and cluster hosts must run byte-identical SQL or their behaviour
//! silently diverges. Tests assert on these strings directly.
//!
//! Three conventions run through all of it, the first two forced by how
//! SurrealDB coerces bound values:
//!
//! - **Never bind JSON `null`.** A `null` into an `option<T>` field is rejected
//!   ("Expected `none | string` but found `NULL`") — `option` means "T or NONE",
//!   and NULL is neither. So absent values are OMITTED from the bound object
//!   rather than nulled, which is why writes go through `CONTENT`/`MERGE` with
//!   an object built in Rust instead of a fixed `SET` list.
//! - **Datetimes must be cast in the statement.** A bound RFC 3339 string won't
//!   coerce into a `datetime` field, and a cast can't live inside a bound
//!   object, hence the `CONTENT object::extend($content, { at: <datetime>$x })`
//!   shape: non-datetime fields come from Rust, datetime fields are spliced in
//!   with their casts. Record links need the same treatment.
//! - **Record ids are always bound as a (table, key) PAIR.** The single-argument
//!   `type::record("_00_schedule:game-sync")` silently truncates at the hyphen
//!   and addresses `_00_schedule:game` instead, so every statement here uses the
//!   two-argument form.
//! - **Every state transition is guarded** by the status it expects to see, so a
//!   duplicate advancement pass, a late-completing killed job, or a second
//!   ticker can't overwrite a decision that was already made.

// ---------------------------------------------------------------------------
// Planning
// ---------------------------------------------------------------------------

/// Schedules that need a fire time computed: never planned, or replanned after
/// a spec change (deploy sets `next_fire_at = NONE` when the hash moves).
pub const SELECT_UNPLANNED: &str = "\
SELECT * FROM _00_schedule \
WHERE next_fire_at = NONE AND paused = false AND config_disabled = false";

/// Store a freshly computed fire time. Guarded on still-being-unplanned so a
/// concurrent planner (or a deploy that just reset the field) wins cleanly.
pub const PLAN_NEXT_FIRE: &str = "\
UPDATE type::record($tb, $key) SET next_fire_at = <datetime>$next, last_error = NONE \
WHERE next_fire_at = NONE RETURN AFTER";

/// Record why a schedule can't be planned (unparseable cron, bad timezone).
/// `next_fire_at` stays NONE, so the schedule is inert until the spec changes.
pub const RECORD_ERROR: &str = "UPDATE type::record($tb, $key) SET last_error = $error";

// ---------------------------------------------------------------------------
// Firing
// ---------------------------------------------------------------------------

/// Schedules due to fire now.
///
/// `paused` deliberately wins over `trigger_requested_at` (below): pausing is
/// the big red button, and an operator who paused a schedule should not be
/// surprised by a queued trigger firing anyway.
pub const SELECT_DUE: &str = "\
SELECT * FROM _00_schedule \
WHERE paused = false AND config_disabled = false \
AND next_fire_at != NONE AND next_fire_at <= time::now()";

/// Schedules with an operator trigger waiting. Separate from `SELECT_DUE` so a
/// trigger fires immediately without disturbing the cron clock.
pub const SELECT_TRIGGERED: &str = "\
SELECT * FROM _00_schedule WHERE paused = false AND trigger_requested_at != NONE";

/// Claim a scheduled fire: compare-and-swap on the fire time we observed.
///
/// An empty result means another ticker (or a redeploy's replan) moved the
/// field first, and this pass must not spawn anything. That is what makes an
/// accidental second ticker harmless rather than a double-fire.
pub const CLAIM_FIRE: &str = "\
UPDATE type::record($tb, $key) SET \
last_fire_at = <datetime>$fire, next_fire_at = <datetime>$next, last_error = NONE \
WHERE next_fire_at = <datetime>$observed RETURN AFTER";

/// Claim an operator trigger: CAS on the trigger stamp, leaving the cron clock
/// (`next_fire_at`) untouched so a manual run doesn't shift the schedule.
pub const CLAIM_TRIGGER: &str = "\
UPDATE type::record($tb, $key) SET \
last_fire_at = <datetime>$fire, trigger_requested_at = NONE, last_error = NONE \
WHERE trigger_requested_at = <datetime>$observed RETURN AFTER";

// ---------------------------------------------------------------------------
// Fan-out concurrency
// ---------------------------------------------------------------------------

/// How many runs for this (schedule, fan-out key) are still in flight. Backed
/// by the `(schedule_name, key, status)` index, so this stays cheap at fan-out
/// widths in the thousands.
pub const COUNT_ACTIVE_RUNS: &str = "\
SELECT VALUE count() FROM _00_schedule_run \
WHERE schedule_name = $name AND key = $key AND status = 'running' GROUP ALL";

/// In-flight runs for a (schedule, key), for `concurrency: replace` to kill.
pub const SELECT_ACTIVE_RUNS: &str = "\
SELECT id, job_id, workflow_run FROM _00_schedule_run \
WHERE schedule_name = $name AND key = $key AND status = 'running'";

// ---------------------------------------------------------------------------
// Spawning
// ---------------------------------------------------------------------------

/// History row for a fire. `CREATE` (not `UPSERT`) on purpose: the deterministic
/// id makes a duplicate the signal that this fire already happened.
///
/// `schedule` is spliced through `type::record` for the same reason datetimes are
/// cast: a bound string won't coerce into a `record<…>` field either.
pub const CREATE_SCHEDULE_RUN: &str = "\
CREATE type::record($tb, $key) CONTENT object::extend($content, \
{ fire_at: <datetime>$fire, schedule: type::record($schedule_tb, $schedule_key) })";

/// Same, for a run that is born terminal — a `skip` records a suppressed tick
/// that never ran. `finished_at` is spliced here rather than put in `$content`
/// because a bound RFC 3339 string will not coerce into a `datetime` field.
pub const CREATE_TERMINAL_SCHEDULE_RUN: &str = "\
CREATE type::record($tb, $key) CONTENT object::extend($content, \
{ fire_at: <datetime>$fire, finished_at: <datetime>$fire, \
schedule: type::record($schedule_tb, $schedule_key) })";

/// Attach the workflow run to its schedule run. A separate statement because an
/// optional record link can't be spliced conditionally into a constant, and the
/// link is a convenience for the CLI rather than something correctness rests on
/// (both ids derive from the same run key).
pub const LINK_WORKFLOW_RUN: &str =
    "UPDATE type::record($tb, $key) SET workflow_run = type::record($wf_tb, $wf_key)";

/// Spawn the atomic job. The outbox schema defaults `status` to `pending`, which
/// is what makes the existing ingest → pickup → runner path take over; the
/// engine never writes job status itself (the runner is the single writer).
pub const CREATE_JOB: &str = "CREATE type::record($tb, $key) CONTENT $content";

/// Terminal state of a spawned job, for the heal pass and the observer. `ONLY`
/// yields NONE (not an error) when the row is gone, which the caller reads as
/// "this job no longer exists".
pub const SELECT_JOB: &str = "SELECT status, result, errors FROM ONLY type::record($tb, $key)";

/// Finalize a schedule run. Guarded on `running` so a heal pass racing the
/// observer, or a late completion after a kill, can't revive or relabel it.
/// `RETURN AFTER` so the caller gets `schedule_name` and `fire_at` back without a
/// second read — it needs both to denormalize the outcome onto the schedule row.
/// An empty result means the guard did not match (already finalized, or killed),
/// which is also exactly when the last-run write must be skipped.
pub const FINALIZE_SCHEDULE_RUN: &str = "\
UPDATE type::record($tb, $key) MERGE object::extend($patch, { finished_at: time::now() }) \
WHERE status = 'running' RETURN AFTER";

// ---------------------------------------------------------------------------
// Workflow runs
// ---------------------------------------------------------------------------

pub const CREATE_WORKFLOW_RUN: &str = "\
CREATE type::record($tb, $key) CONTENT object::extend($content, { schedule: type::record($schedule_tb, $schedule_key) })";

/// Workflow run with no owning schedule (an ad-hoc `spky workflows trigger`).
pub const CREATE_WORKFLOW_RUN_ADHOC: &str = "CREATE type::record($tb, $key) CONTENT $content";

/// One step row per DAG node. Roots start `blocked` like every other step and
/// are promoted by the same readiness pass, so spawn and advancement share one
/// code path — and one compare-and-swap — instead of two.
pub const CREATE_STEP_RUN: &str = "\
CREATE type::record($tb, $key) CONTENT object::extend($content, \
{ workflow_run: type::record($wf_tb, $wf_key) })";

pub const SELECT_WORKFLOW_RUN: &str = "SELECT * FROM ONLY type::record($tb, $key)";

/// Ask the engine to kill a run. The CLI writes this flag and nothing else; the
/// engine consumes it, so run status stays engine-owned.
pub const REQUEST_WORKFLOW_KILL: &str =
    "UPDATE type::record($tb, $key) SET kill_requested = true WHERE status = 'running'";

pub const SELECT_STEP_RUNS: &str = "\
SELECT id, step, depends_on, status, job_id, output FROM _00_step_run \
WHERE workflow_run = type::record($wf_tb, $wf_key)";

/// Promote a step to `ready`. Winning this CAS is the right to dispatch it, so
/// two concurrent advancement passes can never both create the step's job.
pub const CLAIM_STEP_READY: &str =
    "UPDATE type::record($tb, $key) SET status = 'ready' WHERE status = 'blocked' RETURN AFTER";

/// Record the job a claimed step dispatched. Tolerates `dispatched` so the heal
/// pass can re-stamp after a crash between creating the job and this write.
pub const MARK_STEP_DISPATCHED: &str = "\
UPDATE type::record($tb, $key) SET status = 'dispatched', job_id = $job_id \
WHERE status INSIDE ['ready', 'dispatched']";

/// Land a step's terminal state (and its captured output). Guarded on
/// `dispatched`: only a step whose job actually ran can report an outcome.
pub const FINALIZE_STEP: &str = "\
UPDATE type::record($tb, $key) MERGE object::extend($patch, { finished_at: time::now() }) \
WHERE status = 'dispatched'";

/// Fail a step that could not be dispatched at all (no outbox table to create its
/// job in). Needs its own statement because such a step is still `ready` — it
/// never reached `dispatched`, so `FINALIZE_STEP` would silently do nothing.
pub const FAIL_UNDISPATCHABLE_STEP: &str = "\
UPDATE type::record($tb, $key) SET status = 'failed', error = $error, finished_at = time::now() \
WHERE status = 'ready'";

/// Skip a step that can never run (an ancestor failed, or the run was killed).
pub const SKIP_STEP: &str = "\
UPDATE type::record($tb, $key) SET status = 'skipped', finished_at = time::now() \
WHERE status INSIDE ['blocked', 'ready']";

pub const FINALIZE_WORKFLOW_RUN: &str = "\
UPDATE type::record($tb, $key) MERGE object::extend($patch, { finished_at: time::now() }) \
WHERE status = 'running'";

/// Runs an operator asked to kill.
pub const SELECT_KILL_REQUESTED: &str =
    "SELECT VALUE id FROM _00_workflow_run WHERE kill_requested = true AND status = 'running'";

// ---------------------------------------------------------------------------
// Observing job completion
// ---------------------------------------------------------------------------

/// Reverse lookup from a job id to whatever the engine spawned it for. The
/// observer sees a job terminal event and needs to know whether it belongs to a
/// schedule run, a workflow step, or nothing at all.
pub const FIND_RUN_BY_JOB: &str =
    "SELECT id FROM _00_schedule_run WHERE job_id = $job_id AND status = 'running'";

pub const FIND_STEP_BY_JOB: &str = "\
SELECT id, workflow_run FROM _00_step_run \
WHERE job_id = $job_id AND status = 'dispatched'";

// ---------------------------------------------------------------------------
// Heal pass
// ---------------------------------------------------------------------------

/// Atomic-job runs whose job row may already have reached a terminal status
/// while the run row is still `running` — i.e. the ingest event that should have
/// notified the engine was lost (SSP restart, dropped `http::post`). Polling for
/// these is the fallback that keeps event delivery best-effort rather than
/// load-bearing.
pub const SELECT_RUNNING_JOB_RUNS: &str = "\
SELECT id, job_id FROM _00_schedule_run \
WHERE status = 'running' AND kind = 'job' AND job_id != NONE";

/// Workflow runs to re-examine for the same reason.
pub const SELECT_RUNNING_WORKFLOW_RUNS: &str =
    "SELECT VALUE id FROM _00_workflow_run WHERE status = 'running'";

// ---------------------------------------------------------------------------
// Retention
// ---------------------------------------------------------------------------

/// Terminal statuses a schedule run can be pruned in. `running` is deliberately
/// absent: an in-flight run is never pruned no matter how long it has been
/// running. Pruning is driven one status at a time (see below), so this is also
/// the iteration order.
pub const SCHEDULE_RUN_PRUNABLE: [&str; 5] =
    ["success", "skipped", "replaced", "failed", "killed"];

/// Terminal statuses of a workflow run.
pub const WORKFLOW_RUN_PRUNABLE: [&str; 3] = ["success", "failed", "killed"];

/// Drop one batch of history past the retention window.
///
/// Three things about this shape are load-bearing, all verified against the
/// embedded SurrealDB the `db_tests` run on:
///
/// * **One status at a time, by equality.** `status != 'running'` is not
///   index-usable, and `status IN [...]` plans as a `UnionIndexScan` whose
///   per-branch access still only covers the status column. A single
///   `status = $status` is one `IndexScan` on `idx_srun_retention`, and it is what
///   makes the `ORDER BY` below sort-free.
/// * **`ORDER BY finished_at ASC LIMIT $batch`.** SurrealDB does NOT push the
///   `finished_at` range into the index scan — that stays a `Filter`. Ordering by
///   the second index column instead walks the index oldest-first, so the batch
///   fills from rows certain to match and the `LIMIT` bounds the work.
/// * **Batched, and projected rather than `RETURN BEFORE`.** Deleting a row is not
///   free: on a synced table each DELETE fires the generated `_00_<t>_delete`
///   event, which POSTs the whole record to `/ingest`. `RETURN BEFORE` would also
///   haul every payload back. Projecting `id, finished_at` and returning
///   `count($doomed)` gives the caller the batch size at a fraction of the bytes.
///
/// Wrapped in `RETURN { ... }` so the whole thing is ONE statement with one
/// result: [`crate::db::count_value`] reads only the first result entry, and a
/// bare `LET` would shift the indices.
/// `$except` holds the schedule names that carry their own `history:` window and
/// are pruned separately by [`PRUNE_SCHEDULE_RUNS_FOR`]. Usually empty, in which
/// case the predicate is trivially true and costs nothing.
///
/// `NOT IN`, not `NOT INSIDE`: SurrealDB accepts `INSIDE` as a synonym for `IN`
/// but has no negated form of it, and the parse error surfaces only as a warning
/// from the prune pass — i.e. retention silently stops working.
pub const PRUNE_SCHEDULE_RUNS: &str = "\
RETURN { \
LET $doomed = (SELECT id, schedule_name, finished_at, \
time::floor(finished_at, 1h) AS bucket FROM _00_schedule_run \
WHERE status = $status AND finished_at != NONE AND finished_at < <datetime>$before \
AND schedule_name NOT IN $except \
ORDER BY finished_at ASC LIMIT $batch); \
DELETE $doomed.id; \
RETURN $doomed; \
}";

/// One schedule's history, on that schedule's own window. `schedule_name` is an
/// extra equality on top of the same status/ordering plan.
pub const PRUNE_SCHEDULE_RUNS_FOR: &str = "\
RETURN { \
LET $doomed = (SELECT id, schedule_name, finished_at, \
time::floor(finished_at, 1h) AS bucket FROM _00_schedule_run \
WHERE status = $status AND schedule_name = $name \
AND finished_at != NONE AND finished_at < <datetime>$before \
ORDER BY finished_at ASC LIMIT $batch); \
DELETE $doomed.id; \
RETURN $doomed; \
}";

/// Prune workflow runs together with their steps, oldest batch first.
///
/// The steps go first and are selected by `workflow_run INSIDE $ids`, which rides
/// `idx_step_wf`. The previous form filtered steps on
/// `workflow_run.status != 'running'` — a record dereference, i.e. a point read
/// per candidate row, on top of a full table scan. Deleting a run's steps in the
/// same statement as the run also makes an orphaned step structurally impossible
/// going forward — which is why there is no separate step-run prune: a step is only
/// ever reachable through its run, and the old independent statement pruned steps
/// BEFORE runs anyway (a run finalizes after its steps, so `run.finished_at >=
/// step.finished_at`), so no database can be carrying orphans.
pub const PRUNE_WORKFLOW_RUNS: &str = "\
RETURN { \
LET $doomed = (SELECT id, schedule_name, finished_at, \
time::floor(finished_at, 1h) AS bucket FROM _00_workflow_run \
WHERE status = $status AND finished_at != NONE AND finished_at < <datetime>$before \
ORDER BY finished_at ASC LIMIT $batch); \
LET $ids = $doomed.id; \
DELETE _00_step_run WHERE workflow_run INSIDE $ids; \
DELETE $ids; \
RETURN $doomed; \
}";



// ---------------------------------------------------------------------------
// Retention policy + job-table prune
// ---------------------------------------------------------------------------

/// The singleton policy row. `ONLY` yields NONE rather than an error when deploy
/// has not written it yet (a fresh database, or a stack upgraded ahead of its
/// CLI), which the engine reads as "use the built-in defaults".
pub const SELECT_RETENTION: &str = "SELECT * FROM ONLY _00_retention:default";

/// Schedules carrying their own retention window.
pub const SELECT_HISTORY_OVERRIDES: &str = "\
SELECT name, history_success_secs, history_failed_secs FROM _00_schedule \
WHERE history_success_secs != NONE OR history_failed_secs != NONE";

/// Prune one batch of terminal rows from an outbox job table.
///
/// `{table}` is interpolated, not bound: a table name cannot be a parameter in
/// SurrealQL. The engine only ever substitutes a name it read from
/// `_00_retention.job_tables` AND validated as a bare identifier, so this is not
/// a place where user input reaches the statement.
///
/// Keyed on `updated_at`, because the outbox table has no `finished_at` and a
/// terminal job's last write IS its terminalization. Never keyed on `created_at`,
/// which is only the queue time.
///
/// `pending` and `processing` can never match: the caller passes one terminal
/// status at a time, and the status list it iterates contains neither.
pub fn prune_job_table(table: &str) -> String {
    format!(
        "RETURN {{ \
         LET $doomed = (SELECT id, updated_at, time::floor(updated_at, 1h) AS bucket \
         FROM {table} \
         WHERE status = $status AND updated_at < <datetime>$before \
         ORDER BY updated_at ASC LIMIT $batch); \
         DELETE $doomed.id; \
         RETURN $doomed; \
         }}"
    )
}

/// Terminal job statuses, shortest-lived bucket first. `pending` and `processing`
/// are absent on purpose — an in-flight job is never pruned at any age.
pub const JOB_PRUNABLE: [&str; 2] = ["success", "failed"];

/// A table name safe to interpolate into a statement.
pub fn is_plain_identifier(name: &str) -> bool {
    !name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

// ---------------------------------------------------------------------------
// Rollup
// ---------------------------------------------------------------------------

/// Fold one batch's worth of counts into an hourly bucket.
///
/// A blind `UPSERT` with `+=` and no read first: the row id is derived from
/// (scope, name, bucket), and `DEFAULT 0` on every counter applies before the `+=`,
/// so a first touch and an accumulate are the same statement. That is what lets two
/// pruners — or a retried pass — fold concurrently without a read-modify-write race.
///
/// One named field per outcome rather than a map, because `+=` on a named field is a
/// single atomic SET while incrementing a computed key inside an object is not
/// expressible in one statement.
pub const UPSERT_ROLLUP: &str = "\
UPSERT type::record($tb, $key) SET scope = $scope, name = $name, \
bucket = <datetime>$bucket, success += $success, failed += $failed, \
skipped += $skipped, replaced += $replaced, killed += $killed";

// ---------------------------------------------------------------------------
// Row cap
// ---------------------------------------------------------------------------

/// How many rows one status holds. Used only by the cap, which runs rarely: this is
/// an index walk, not a point read.
pub const COUNT_RUNS_IN_STATUS: &str = "\
SELECT VALUE count() FROM _00_schedule_run WHERE status = $status GROUP ALL";

/// Same for an outbox table.
pub fn count_jobs_in_status(table: &str) -> String {
    format!("SELECT VALUE count() FROM {table} WHERE status = $status GROUP ALL")
}

// ---------------------------------------------------------------------------
// Last-run denormalization
// ---------------------------------------------------------------------------

/// Record a run's outcome on its schedule so `spky schedules list` needs neither a
/// scan of the run table nor the run row to still exist.
///
/// `$at` is the run's `fire_at`, not its `finished_at`, and the guard is what keeps
/// this cheap on a wide fan-out: every run of one fire shares a `fire_at`, so after
/// the first outcome lands the guard stops matching and the remaining N-1 items are
/// no-ops rather than N writes to the same hot row.
///
/// Three clauses, in order: nothing recorded yet; a newer fire (so a late heal pass
/// reconciling an old run can never overwrite a newer outcome); or the SAME fire
/// reporting a failure, because one status cannot summarise a fan-out and a failure
/// is the outcome an operator needs to see.
///
/// A no-op when the schedule row is gone — `UPDATE` on a missing record does not
/// create one, so pruning or deleting a schedule cannot resurrect a partial row.
pub const SET_LAST_RUN: &str = "\
UPDATE type::record($tb, $key) SET last_run_status = $status, \
last_run_at = <datetime>$at \
WHERE last_run_at = NONE OR last_run_at < <datetime>$at \
OR (last_run_at = <datetime>$at AND $status IN ['failed', 'killed'])";

#[cfg(test)]
mod tests {
    use super::*;

    /// The shipped DDL, so these assertions are about what actually deploys.
    const SCHEDULE_TABLES: &str = include_str!("../../../apps/cli/src/schedule_tables.surql");

    /// Pull the `ASSERT $value INSIDE [...]` status list for one table out of the DDL.
    fn ddl_statuses(table: &str) -> Vec<String> {
        let field = format!("DEFINE FIELD OVERWRITE status ON TABLE {table} TYPE string");
        let at = SCHEDULE_TABLES
            .find(&field)
            .unwrap_or_else(|| panic!("no status field for {table} in the shipped DDL"));
        let rest = &SCHEDULE_TABLES[at..];
        let open = rest.find('[').expect("status ASSERT has a list");
        let close = rest.find(']').expect("status ASSERT list closes");
        rest[open + 1..close]
            .split(',')
            .map(|s| s.trim().trim_matches('\'').to_string())
            .filter(|s| !s.is_empty())
            .collect()
    }

    /// A status the DDL allows but the prune list omits is a row that is never
    /// collected — a silent, permanent leak. A status the prune list names but the
    /// DDL rejects is a prune that matches nothing. Both are invisible at runtime,
    /// so the two lists are pinned to each other here.
    ///
    /// This is the drift that adding a status to the DDL alone would introduce.
    #[test]
    fn prunable_statuses_cover_every_terminal_status_the_ddl_allows() {
        for (table, prunable) in [
            ("_00_schedule_run", &SCHEDULE_RUN_PRUNABLE[..]),
            ("_00_workflow_run", &WORKFLOW_RUN_PRUNABLE[..]),
        ] {
            let mut ddl = ddl_statuses(table);
            ddl.sort();
            let mut expected: Vec<String> =
                prunable.iter().map(|s| s.to_string()).chain(["running".to_string()]).collect();
            expected.sort();
            assert_eq!(
                ddl, expected,
                "{table}: the DDL's allowed statuses and (prunable + running) disagree. \
                 A status the DDL allows but the prune list omits leaks forever; one the \
                 prune list names but the DDL rejects prunes nothing."
            );
        }
    }

    /// `running` is the only non-terminal schedule-run status, and it is the one the
    /// concurrency count and the heal pass select on. If the DDL ever renamed it,
    /// `COUNT_ACTIVE_RUNS` would quietly count zero and `concurrency: skip` would
    /// stop suppressing.
    #[test]
    fn the_non_terminal_status_the_ddl_defines_is_the_one_the_engine_selects_on() {
        for table in ["_00_schedule_run", "_00_workflow_run"] {
            assert!(
                ddl_statuses(table).contains(&"running".to_string()),
                "{table} must allow 'running'"
            );
        }
        assert!(COUNT_ACTIVE_RUNS.contains("status = 'running'"));
        assert!(SELECT_ACTIVE_RUNS.contains("status = 'running'"));
        assert!(SELECT_RUNNING_JOB_RUNS.contains("status = 'running'"));
        assert!(SELECT_RUNNING_WORKFLOW_RUNS.contains("status = 'running'"));
    }

    /// Every status this crate writes into a step row must be one the DDL allows.
    /// A rejected write is not an error the engine surfaces — the statement is
    /// guarded, so "rejected by ASSERT" and "no row matched the guard" look
    /// identical, and the DAG would just stop advancing.
    #[test]
    fn every_step_status_the_engine_writes_is_allowed_by_the_ddl() {
        let allowed = ddl_statuses("_00_step_run");
        for status in ["blocked", "ready", "dispatched", "success", "failed", "skipped"] {
            assert!(
                allowed.contains(&status.to_string()),
                "_00_step_run rejects '{status}', which the engine writes"
            );
        }
        // And the statements that write them still name those statuses.
        assert!(CLAIM_STEP_READY.contains("'ready'") && CLAIM_STEP_READY.contains("'blocked'"));
        assert!(MARK_STEP_DISPATCHED.contains("'dispatched'"));
        assert!(SKIP_STEP.contains("'skipped'"));
        assert!(FAIL_UNDISPATCHABLE_STEP.contains("'failed'"));
    }

    /// Every index the engine's statements rely on must exist in the shipped DDL.
    /// Losing one does not fail a query — it silently turns an IndexScan into a
    /// TableScan, which at fan-out width is the difference between a bounded prune
    /// and a whole-table sweep every minute.
    #[test]
    fn the_indexes_the_statements_depend_on_are_defined() {
        for index in [
            "idx_srun_active",     // COUNT_ACTIVE_RUNS / SELECT_ACTIVE_RUNS
            "idx_srun_job",        // FIND_RUN_BY_JOB
            "idx_srun_retention",  // PRUNE_SCHEDULE_RUNS status + ordering
            "idx_srun_fire",       // `spky schedules` fire_at sorts
            "idx_wfrun_retention", // PRUNE_WORKFLOW_RUNS
            "idx_step_wf",         // PRUNE_WORKFLOW_RUNS' step delete
        ] {
            assert!(
                SCHEDULE_TABLES.contains(index),
                "{index} is gone from the DDL but statements still assume it"
            );
        }
    }

    /// The retention indexes must lead with `status` and follow with the column the
    /// prune orders by. Reversing the columns still parses, still returns correct
    /// rows, and silently loses both the index scan and the sort-free ordering.
    #[test]
    fn retention_indexes_are_ordered_status_then_timestamp() {
        for (index, table) in [
            ("idx_srun_retention", "_00_schedule_run"),
            ("idx_wfrun_retention", "_00_workflow_run"),
        ] {
            let at = SCHEDULE_TABLES.find(index).expect("index defined");
            let decl = &SCHEDULE_TABLES[at..];
            let end = decl.find(';').expect("index declaration ends");
            let decl = &decl[..end];
            assert!(decl.contains(table), "{index} must be on {table}: {decl}");
            let cols = decl.find("COLUMNS").expect("index has columns");
            let cols = &decl[cols..];
            assert!(
                cols.replace(' ', "").contains("status,finished_at"),
                "{index} must be (status, finished_at) in that order: {cols}"
            );
        }
    }

    /// Every mutation the engine issues must be guarded by the state it expects,
    /// or a concurrent pass could clobber a decision already made.
    #[test]
    fn state_transitions_are_guarded() {
        for (name, sql) in [
            ("PLAN_NEXT_FIRE", PLAN_NEXT_FIRE),
            ("CLAIM_FIRE", CLAIM_FIRE),
            ("CLAIM_TRIGGER", CLAIM_TRIGGER),
            ("FINALIZE_SCHEDULE_RUN", FINALIZE_SCHEDULE_RUN),
            ("CLAIM_STEP_READY", CLAIM_STEP_READY),
            ("MARK_STEP_DISPATCHED", MARK_STEP_DISPATCHED),
            ("FINALIZE_STEP", FINALIZE_STEP),
            ("FAIL_UNDISPATCHABLE_STEP", FAIL_UNDISPATCHABLE_STEP),
            ("SKIP_STEP", SKIP_STEP),
            ("FINALIZE_WORKFLOW_RUN", FINALIZE_WORKFLOW_RUN),
        ] {
            assert!(sql.contains(" WHERE "), "{name} must carry a WHERE guard");
        }
    }

    /// The engine must never write outbox `status` — the JobRunner is the single
    /// status writer. It only ever CREATEs a row and lets the schema default it.
    #[test]
    fn engine_never_writes_job_status() {
        assert!(CREATE_JOB.starts_with("CREATE "));
        assert!(!CREATE_JOB.contains("status"));
    }

    /// The single-argument `type::record` truncates keys at a hyphen, so no
    /// statement may ever use it.
    #[test]
    fn record_ids_are_bound_as_table_and_key_pairs() {
        for sql in [
            SELECT_UNPLANNED, PLAN_NEXT_FIRE, RECORD_ERROR, SELECT_DUE, SELECT_TRIGGERED,
            CLAIM_FIRE, CLAIM_TRIGGER, COUNT_ACTIVE_RUNS, SELECT_ACTIVE_RUNS,
            CREATE_SCHEDULE_RUN, CREATE_TERMINAL_SCHEDULE_RUN, LINK_WORKFLOW_RUN, CREATE_JOB,
            SELECT_JOB,
            FINALIZE_SCHEDULE_RUN, CREATE_WORKFLOW_RUN, CREATE_STEP_RUN, SELECT_WORKFLOW_RUN,
            REQUEST_WORKFLOW_KILL, SELECT_STEP_RUNS, CLAIM_STEP_READY, MARK_STEP_DISPATCHED,
            FINALIZE_STEP, FAIL_UNDISPATCHABLE_STEP, SKIP_STEP, FINALIZE_WORKFLOW_RUN,
            SELECT_KILL_REQUESTED, FIND_RUN_BY_JOB, FIND_STEP_BY_JOB,
            SELECT_RUNNING_JOB_RUNS, SELECT_RUNNING_WORKFLOW_RUNS,
            PRUNE_SCHEDULE_RUNS, PRUNE_WORKFLOW_RUNS,
        ] {
            for (i, _) in sql.match_indices("type::record($") {
                let call = &sql[i..];
                let end = call.find(')').expect("closing paren");
                assert!(
                    call[..end].contains(", $"),
                    "single-argument type::record truncates hyphenated keys: {}",
                    &call[..end]
                );
            }
        }
    }

    /// A bound RFC 3339 string does not coerce into a `datetime` field, so every
    /// datetime the engine writes has to be cast in the statement itself.
    #[test]
    fn datetimes_are_cast_in_the_statement() {
        for sql in [
            PLAN_NEXT_FIRE,
            CLAIM_FIRE,
            CLAIM_TRIGGER,
            CREATE_SCHEDULE_RUN,
            CREATE_TERMINAL_SCHEDULE_RUN,
        ] {
            assert!(sql.contains("<datetime>$"), "missing datetime cast in: {sql}");
        }
        for sql in [PRUNE_SCHEDULE_RUNS, PRUNE_WORKFLOW_RUNS] {
            assert!(sql.contains("<datetime>$before"), "missing datetime cast in: {sql}");
        }
    }

    /// Pruning must never touch in-flight work. The guard moved from a
    /// `status != 'running'` predicate to the status lists the caller iterates, so
    /// the invariant is asserted on those.
    #[test]
    fn prunes_skip_running_rows() {
        assert!(!SCHEDULE_RUN_PRUNABLE.contains(&"running"));
        assert!(!WORKFLOW_RUN_PRUNABLE.contains(&"running"));
        for sql in [PRUNE_SCHEDULE_RUNS, PRUNE_WORKFLOW_RUNS] {
            assert!(sql.contains("status = $status"), "prune must bind one status: {sql}");
            assert!(!sql.contains("'running'"), "prune must not name a status inline: {sql}");
        }
    }

    /// Every prune is one statement (so `count_value` reads its count) and is
    /// bounded by a batch limit (so a backlog drains without one giant transaction
    /// full of per-row ingest events).
    #[test]
    fn prunes_are_single_batched_statements() {
        for sql in [PRUNE_SCHEDULE_RUNS, PRUNE_WORKFLOW_RUNS] {
            assert!(sql.starts_with("RETURN {"), "prune must be one block: {sql}");
            assert!(sql.contains("LIMIT $batch"), "prune must be batched: {sql}");
            assert!(!sql.contains("RETURN BEFORE"), "prune must not haul rows back: {sql}");
        }
    }

    /// No prune statement may use `NOT INSIDE`. SurrealDB parses `INSIDE` as a
    /// synonym for `IN` but has NO negated form, and a parse error inside the prune
    /// pass surfaces only as a warning — so retention stops working and nothing
    /// says so. This caught exactly that.
    #[test]
    fn prunes_use_the_negated_operator_that_actually_parses() {
        for sql in [PRUNE_SCHEDULE_RUNS, PRUNE_SCHEDULE_RUNS_FOR, PRUNE_WORKFLOW_RUNS] {
            assert!(!sql.contains("NOT INSIDE"), "`NOT INSIDE` does not parse: {sql}");
        }
    }

    /// Interpolated table names are checked, because a table name cannot be bound
    /// as a parameter. Only names read back from `_00_retention.job_tables` ever
    /// reach the statement, and they still have to look like identifiers.
    #[test]
    fn only_plain_identifiers_are_interpolated_into_the_job_prune() {
        assert!(is_plain_identifier("job"));
        assert!(is_plain_identifier("statistics_job2"));
        assert!(is_plain_identifier("_internal"));
        assert!(!is_plain_identifier(""));
        assert!(!is_plain_identifier("2job"));
        assert!(!is_plain_identifier("job; DELETE user"));
        assert!(!is_plain_identifier("job-table"));
        assert!(!is_plain_identifier("job table"));
    }

    /// The job prune keys off `updated_at`, never `created_at`: `created_at` is only
    /// the queue time, so a job that waited on a delay or burned retries would be
    /// judged by when it was enqueued rather than when it finished.
    #[test]
    fn the_job_prune_ages_rows_by_their_last_write() {
        let sql = prune_job_table("job");
        assert!(sql.contains("updated_at < <datetime>$before"));
        assert!(sql.contains("ORDER BY updated_at ASC"));
        assert!(!sql.contains("created_at"));
        assert!(sql.contains("LIMIT $batch"));
    }

    /// In-flight jobs are never prunable, whatever their age.
    #[test]
    fn the_job_prune_never_names_a_non_terminal_status() {
        assert!(!JOB_PRUNABLE.contains(&"pending"));
        assert!(!JOB_PRUNABLE.contains(&"processing"));
        assert_eq!(JOB_PRUNABLE.len(), 2);
    }

    /// The last-run write is guarded on its timestamp, so a heal pass reconciling an
    /// old run cannot overwrite a newer outcome with a staler one.
    #[test]
    fn the_last_run_write_cannot_go_backwards() {
        assert!(SET_LAST_RUN.contains("last_run_at = NONE OR last_run_at < <datetime>$at"));
        // A failure within the same fire still wins, so a fan-out where one item
        // fails does not report success.
        assert!(SET_LAST_RUN.contains("$status IN ['failed', 'killed']"));
    }

    /// The finalize has to hand back the row, or the caller cannot denormalize the
    /// outcome without a second read.
    #[test]
    fn finalizing_a_run_returns_the_row() {
        assert!(FINALIZE_SCHEDULE_RUN.contains("RETURN AFTER"));
    }

    /// Every field the engine reads out of `_00_retention` must exist in the shipped
    /// DDL, and vice versa. A field the engine reads but deploy never defines silently
    /// falls back to a default — retention would look configured and not be.
    #[test]
    fn the_retention_row_defines_exactly_what_the_engine_reads() {
        for field in [
            "success_secs",
            "failed_secs",
            "run_success_secs",
            "run_failed_secs",
            "job_tables",
            "max_rows",
        ] {
            assert!(
                SCHEDULE_TABLES.contains(&format!("{field} ON TABLE _00_retention")),
                "_00_retention is missing `{field}`, which the prune pass reads"
            );
        }
        assert!(SCHEDULE_TABLES.contains("DEFINE TABLE OVERWRITE _00_retention SCHEMAFULL"));
        // Root-only: the policy decides what gets deleted, so it must never be
        // client-reachable.
        assert!(SCHEDULE_TABLES.contains("_00_retention SCHEMAFULL PERMISSIONS NONE"));
    }

    /// The per-schedule override fields the engine selects on have to exist too, or
    /// `SELECT ... WHERE history_success_secs != NONE` fails and every override is
    /// silently ignored.
    #[test]
    fn the_schedule_row_defines_the_history_override_fields() {
        for field in ["history_success_secs", "history_failed_secs"] {
            assert!(
                SCHEDULE_TABLES.contains(&format!("{field} ON TABLE _00_schedule")),
                "_00_schedule is missing `{field}`"
            );
            assert!(SELECT_HISTORY_OVERRIDES.contains(field));
        }
    }

    /// Same for the denormalized last-run fields the CLI now reads instead of
    /// scanning the run table.
    #[test]
    fn the_schedule_row_defines_the_last_run_fields() {
        for field in ["last_run_status", "last_run_at"] {
            assert!(
                SCHEDULE_TABLES.contains(&format!("{field} ON TABLE _00_schedule")),
                "_00_schedule is missing `{field}`"
            );
            assert!(SET_LAST_RUN.contains(field));
        }
    }

    /// Every counter the rollup fold writes must exist on `_00_run_rollup`, and the
    /// statement must name all five — one moves per fold, but a SCHEMAFULL table
    /// rejects the whole statement if any is undefined, and an unbound parameter is a
    /// query error rather than a zero.
    #[test]
    fn the_rollup_defines_every_counter_the_fold_writes() {
        for counter in ["success", "failed", "skipped", "replaced", "killed"] {
            assert!(
                SCHEDULE_TABLES.contains(&format!("{counter} ON TABLE _00_run_rollup")),
                "_00_run_rollup is missing the `{counter}` counter"
            );
            assert!(
                UPSERT_ROLLUP.contains(&format!("{counter} += ${counter}")),
                "the fold must accumulate `{counter}`, not overwrite it"
            );
        }
        // Root-only, and indexed for the lookup the CLI does.
        assert!(SCHEDULE_TABLES.contains("_00_run_rollup SCHEMAFULL PERMISSIONS NONE"));
        assert!(SCHEDULE_TABLES.contains("idx_rollup_lookup"));
    }

    /// The fold accumulates; it must never CONTENT/SET a counter outright, or a second
    /// prune into the same hour would discard the first one's count.
    #[test]
    fn the_rollup_fold_is_an_accumulate_not_a_replace() {
        assert!(UPSERT_ROLLUP.starts_with("UPSERT "));
        assert!(!UPSERT_ROLLUP.contains("CONTENT"));
        // The identity fields ARE set outright, which is correct: they are the key.
        assert!(UPSERT_ROLLUP.contains("scope = $scope"));
        assert!(UPSERT_ROLLUP.contains("bucket = <datetime>$bucket"));
    }

    /// Every prune projects the hour bucket the fold groups by. Without it the fold
    /// silently counts nothing — the prune still deletes, so the rows are simply gone
    /// and uncounted.
    #[test]
    fn prunes_project_the_bucket_the_rollup_folds_by() {
        for sql in [
            PRUNE_SCHEDULE_RUNS.to_string(),
            PRUNE_SCHEDULE_RUNS_FOR.to_string(),
            PRUNE_WORKFLOW_RUNS.to_string(),
            prune_job_table("job"),
        ] {
            assert!(sql.contains("AS bucket"), "no bucket projected: {sql}");
            assert!(sql.contains("time::floor("), "bucket must be hour-floored: {sql}");
        }
        // And the run prunes carry the name the bucket is attributed to.
        for sql in [PRUNE_SCHEDULE_RUNS, PRUNE_SCHEDULE_RUNS_FOR, PRUNE_WORKFLOW_RUNS] {
            assert!(sql.contains("schedule_name"), "no name projected: {sql}");
        }
    }

    /// Every prune statement must project the column it orders by. SurrealDB v3
    /// rejects `ORDER BY` on a field absent from the projection, and the failure
    /// surfaces only as a warning from the prune pass.
    #[test]
    fn prunes_project_the_column_they_order_by() {
        for (sql, col) in [
            (PRUNE_SCHEDULE_RUNS.to_string(), "finished_at"),
            (PRUNE_SCHEDULE_RUNS_FOR.to_string(), "finished_at"),
            (PRUNE_WORKFLOW_RUNS.to_string(), "finished_at"),
            (prune_job_table("job"), "updated_at"),
        ] {
            assert!(sql.contains(&format!("ORDER BY {col} ASC")), "{sql}");
            let select = &sql[sql.find("SELECT").expect("has a SELECT")..];
            let projection = &select[..select.find("FROM").expect("has a FROM")];
            assert!(
                projection.contains(col),
                "`{col}` must be in the projection to be ordered by: {projection}"
            );
        }
    }

    /// The run prunes walk the index oldest-first. Without the ordering the
    /// `finished_at` range degrades to a filter over the whole status partition.
    #[test]
    fn run_prunes_are_index_ordered() {
        assert!(PRUNE_SCHEDULE_RUNS.contains("ORDER BY finished_at ASC"));
        assert!(PRUNE_WORKFLOW_RUNS.contains("ORDER BY finished_at ASC"));
    }
}
