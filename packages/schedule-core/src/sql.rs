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
pub const FINALIZE_SCHEDULE_RUN: &str = "\
UPDATE type::record($tb, $key) MERGE object::extend($patch, { finished_at: time::now() }) \
WHERE status = 'running'";

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

/// Drop history past the retention window. Terminal rows only — an in-flight run
/// is never pruned no matter how long it has been running. `RETURN BEFORE`
/// because a plain DELETE reports nothing, and a silent prune is indistinguishable
/// from a broken one.
pub const PRUNE_SCHEDULE_RUNS: &str = "\
DELETE _00_schedule_run \
WHERE status != 'running' AND finished_at != NONE AND finished_at < <datetime>$before \
RETURN BEFORE";

pub const PRUNE_STEP_RUNS: &str = "\
DELETE _00_step_run WHERE finished_at != NONE AND finished_at < <datetime>$before \
AND workflow_run.status != 'running' RETURN BEFORE";

pub const PRUNE_WORKFLOW_RUNS: &str = "\
DELETE _00_workflow_run \
WHERE status != 'running' AND finished_at != NONE AND finished_at < <datetime>$before \
RETURN BEFORE";

#[cfg(test)]
mod tests {
    use super::*;

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
            PRUNE_SCHEDULE_RUNS, PRUNE_STEP_RUNS, PRUNE_WORKFLOW_RUNS,
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
        for sql in [PRUNE_SCHEDULE_RUNS, PRUNE_STEP_RUNS, PRUNE_WORKFLOW_RUNS] {
            assert!(sql.contains("<datetime>$before"), "missing datetime cast in: {sql}");
        }
    }

    /// Pruning must never touch in-flight work.
    #[test]
    fn prunes_skip_running_rows() {
        assert!(PRUNE_SCHEDULE_RUNS.contains("status != 'running'"));
        assert!(PRUNE_WORKFLOW_RUNS.contains("status != 'running'"));
        assert!(PRUNE_STEP_RUNS.contains("workflow_run.status != 'running'"));
    }
}
