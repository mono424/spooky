-- ==================================================
-- SPOOKY SCHEDULING TABLES
-- Server-side cron / workflow definitions and their run history. Split out of
-- meta_tables_remote.surql so `packages/schedule-core` can include_str! exactly
-- the DDL that ships and run its engine tests against it — the rest of the
-- internal schema needs CLI-time substitution (the surrealism `mod::` call) and
-- won't parse standalone.
-- Applied by the same paths as the other meta tables: migrate::apply_internal_schema
-- (VM/cloud) and schema_builder::build_server_schema (free/Cloudflare push).
-- ==================================================

-- ==================================================
-- SPOOKY SCHEDULE (declarative server-side cron / workflow definitions)
-- One row per `schedules:` / `workflows:` entry in sp00ky.yml, keyed by name
-- (_00_schedule:<name>). Root-only: a schedule can fan out over arbitrary
-- SurrealQL and spawn jobs against any backend, so it must never be
-- client-reachable.
--
-- Three writers, three disjoint field groups, and they must stay disjoint:
--   deploy   (`spky deploy` / `spky dev`) writes the SPEC fields + spec_hash,
--            and resets next_fire_at to NONE only when spec_hash changed.
--            It NEVER writes paused — an operator pause must survive a deploy.
--   operator (`spky schedules pause|resume|trigger`) writes paused and
--            trigger_requested_at, and nothing else.
--   engine   (the schedule sweep, singlenode SSP or cluster scheduler) writes
--            next_fire_at / last_fire_at / last_error, and nothing else.
-- trigger_requested_at is its own field rather than "just set next_fire_at to
-- now" precisely so an operator trigger can never collide with the engine's
-- compare-and-swap on the cron clock.
-- ==================================================
DEFINE TABLE OVERWRITE _00_schedule SCHEMAFULL PERMISSIONS NONE;

-- Spec: written by deploy only.
DEFINE FIELD OVERWRITE name ON TABLE _00_schedule TYPE string;
DEFINE FIELD OVERWRITE kind ON TABLE _00_schedule TYPE string
    ASSERT $value INSIDE ['job', 'workflow'];
-- Exactly one of cron / every_ms is set (the CLI validates; the engine records
-- a last_error and stays inert if both or neither arrive).
DEFINE FIELD OVERWRITE cron ON TABLE _00_schedule TYPE option<string>;
DEFINE FIELD OVERWRITE every_ms ON TABLE _00_schedule TYPE option<int>;
-- IANA name; NONE means UTC. Cron is evaluated in this zone, so "daily at
-- 03:00" stays 03:00 local across DST transitions.
DEFINE FIELD OVERWRITE timezone ON TABLE _00_schedule TYPE option<string>;
-- Outbox table the spawned job rows are created in (resolved from the app's
-- `method.table` at deploy time, so the engine never reads sp00ky.yml).
DEFINE FIELD OVERWRITE target_table ON TABLE _00_schedule TYPE option<string>;
DEFINE FIELD OVERWRITE path ON TABLE _00_schedule TYPE option<string>;
DEFINE FIELD OVERWRITE payload ON TABLE _00_schedule TYPE option<object> FLEXIBLE;
-- Frozen DAG definition for kind = 'workflow': { steps: [...], on_failure }.
DEFINE FIELD OVERWRITE workflow ON TABLE _00_schedule TYPE option<object> FLEXIBLE;
-- Fan-out: a SurrealQL SELECT run on every fire; one run is spawned per row.
DEFINE FIELD OVERWRITE for_each ON TABLE _00_schedule TYPE option<string>;
-- Row field whose value keys the per-item concurrency check. Defaults to `id`.
DEFINE FIELD OVERWRITE for_each_key ON TABLE _00_schedule TYPE option<string>;
DEFINE FIELD OVERWRITE concurrency ON TABLE _00_schedule TYPE string DEFAULT 'skip'
    ASSERT $value INSIDE ['skip', 'allow', 'replace'];
DEFINE FIELD OVERWRITE max_retries ON TABLE _00_schedule TYPE option<int>;
DEFINE FIELD OVERWRITE retry_strategy ON TABLE _00_schedule TYPE option<string>;
DEFINE FIELD OVERWRITE timeout ON TABLE _00_schedule TYPE option<int>;
-- Hash of the spec fields. Deploy skips the UPSERT when unchanged, and resets
-- next_fire_at when it moves so the engine replans against the new cadence.
DEFINE FIELD OVERWRITE spec_hash ON TABLE _00_schedule TYPE option<string>;
-- `enabled: false` in config. Distinct from `paused` so config intent and
-- operator intent can't overwrite each other.
DEFINE FIELD OVERWRITE config_disabled ON TABLE _00_schedule TYPE bool DEFAULT false;

-- Operator: written by the CLI only.
DEFINE FIELD OVERWRITE paused ON TABLE _00_schedule TYPE bool DEFAULT false;
DEFINE FIELD OVERWRITE trigger_requested_at ON TABLE _00_schedule TYPE option<datetime>;

-- Engine: written by the schedule sweep only.
DEFINE FIELD OVERWRITE next_fire_at ON TABLE _00_schedule TYPE option<datetime>;
DEFINE FIELD OVERWRITE last_fire_at ON TABLE _00_schedule TYPE option<datetime>;
-- Why the schedule couldn't be planned or fanned out (bad cron, failing
-- forEach query). Surfaced by `spky schedules list`.
DEFINE FIELD OVERWRITE last_error ON TABLE _00_schedule TYPE option<string>;

DEFINE FIELD OVERWRITE created_at ON TABLE _00_schedule TYPE datetime DEFAULT time::now() READONLY;
DEFINE FIELD OVERWRITE updated_at ON TABLE _00_schedule TYPE datetime VALUE time::now();

-- ==================================================
-- SPOOKY SCHEDULE RUN (history + per-key concurrency state)
-- One row per fire per fan-out item. The record id is derived from
-- (schedule, fire time, hashed key), which is what makes firing idempotent:
-- two tickers racing the same fire collide on the id instead of double-
-- spawning, and a duplicate CREATE is read as "already fired".
--
-- This table IS the concurrency state — `skip` asks it whether a run for this
-- key is still 'running' rather than consulting in-memory state, so the policy
-- survives restarts and works identically in singlenode and cluster mode.
-- schedule_name is denormalized so history outlives a deleted schedule.
-- ==================================================
DEFINE TABLE OVERWRITE _00_schedule_run SCHEMAFULL PERMISSIONS NONE;

DEFINE FIELD OVERWRITE schedule ON TABLE _00_schedule_run TYPE option<record<_00_schedule>>;
DEFINE FIELD OVERWRITE schedule_name ON TABLE _00_schedule_run TYPE string;
-- '' when the schedule doesn't fan out.
DEFINE FIELD OVERWRITE key ON TABLE _00_schedule_run TYPE string DEFAULT '';
DEFINE FIELD OVERWRITE fire_at ON TABLE _00_schedule_run TYPE datetime;
DEFINE FIELD OVERWRITE kind ON TABLE _00_schedule_run TYPE string
    ASSERT $value INSIDE ['job', 'workflow'];
DEFINE FIELD OVERWRITE status ON TABLE _00_schedule_run TYPE string DEFAULT 'running'
    ASSERT $value INSIDE ['running', 'success', 'failed', 'skipped', 'replaced', 'killed'];
-- 'cron' | 'manual' — how this fire was initiated.
DEFINE FIELD OVERWRITE trigger ON TABLE _00_schedule_run TYPE option<string>;
-- Spawned outbox job (kind = 'job'). A string, not a record link: the outbox
-- table varies per schedule, so a typed link isn't expressible here.
DEFINE FIELD OVERWRITE job_id ON TABLE _00_schedule_run TYPE option<string>;
DEFINE FIELD OVERWRITE workflow_run ON TABLE _00_schedule_run TYPE option<record<_00_workflow_run>>;
-- Snapshot of the forEach row this run was spawned for.
DEFINE FIELD OVERWRITE row ON TABLE _00_schedule_run TYPE option<object> FLEXIBLE;
DEFINE FIELD OVERWRITE error ON TABLE _00_schedule_run TYPE option<object> FLEXIBLE;
DEFINE FIELD OVERWRITE created_at ON TABLE _00_schedule_run TYPE datetime DEFAULT time::now() READONLY;
DEFINE FIELD OVERWRITE updated_at ON TABLE _00_schedule_run TYPE datetime VALUE time::now();
DEFINE FIELD OVERWRITE finished_at ON TABLE _00_schedule_run TYPE option<datetime>;

-- Backs the per-key concurrency count on every fire of a fanned-out schedule.
DEFINE INDEX OVERWRITE idx_srun_active ON TABLE _00_schedule_run
    COLUMNS schedule_name, key, status;
-- Reverse lookup for the job-terminal observer.
DEFINE INDEX OVERWRITE idx_srun_job ON TABLE _00_schedule_run COLUMNS job_id;

-- ==================================================
-- SPOOKY WORKFLOW RUN (one DAG execution)
-- `dag` is a FROZEN copy of the definition taken at spawn, so redeploying a
-- workflow mid-run can never change the semantics of runs already in flight.
-- kill_requested is the operator's channel: the engine consumes it, sets the
-- run terminal, and kills the dispatched steps' jobs — the CLI never writes
-- run status itself.
-- ==================================================
DEFINE TABLE OVERWRITE _00_workflow_run SCHEMAFULL PERMISSIONS NONE;

DEFINE FIELD OVERWRITE schedule ON TABLE _00_workflow_run TYPE option<record<_00_schedule>>;
DEFINE FIELD OVERWRITE schedule_name ON TABLE _00_workflow_run TYPE option<string>;
DEFINE FIELD OVERWRITE workflow_name ON TABLE _00_workflow_run TYPE string;
DEFINE FIELD OVERWRITE dag ON TABLE _00_workflow_run TYPE object FLEXIBLE;
-- Default outbox table for steps that don't name one. Copied from the schedule at
-- spawn (rather than read back through the link) so deleting the schedule can't
-- strand a run mid-flight.
DEFINE FIELD OVERWRITE target_table ON TABLE _00_workflow_run TYPE option<string>;
-- forEach row or manual trigger payload; reachable from every step's payload.
DEFINE FIELD OVERWRITE input ON TABLE _00_workflow_run TYPE option<object> FLEXIBLE;
DEFINE FIELD OVERWRITE status ON TABLE _00_workflow_run TYPE string DEFAULT 'running'
    ASSERT $value INSIDE ['running', 'success', 'failed', 'killed'];
DEFINE FIELD OVERWRITE kill_requested ON TABLE _00_workflow_run TYPE bool DEFAULT false;
DEFINE FIELD OVERWRITE error ON TABLE _00_workflow_run TYPE option<object> FLEXIBLE;
DEFINE FIELD OVERWRITE created_at ON TABLE _00_workflow_run TYPE datetime DEFAULT time::now() READONLY;
DEFINE FIELD OVERWRITE updated_at ON TABLE _00_workflow_run TYPE datetime VALUE time::now();
DEFINE FIELD OVERWRITE finished_at ON TABLE _00_workflow_run TYPE option<datetime>;

DEFINE INDEX OVERWRITE idx_wfrun_status ON TABLE _00_workflow_run COLUMNS status;
DEFINE INDEX OVERWRITE idx_wfrun_sched ON TABLE _00_workflow_run COLUMNS schedule_name;

-- ==================================================
-- SPOOKY STEP RUN (one node of one workflow run)
-- status drives the whole DAG: a step goes blocked -> ready -> dispatched ->
-- success/failed, or straight to skipped when an ancestor fails. Advancement
-- is idempotent because the blocked -> ready promotion is a compare-and-swap:
-- whichever pass wins it is the one that creates the step's job, so two
-- concurrent advancements can't double-dispatch.
-- depends_on is frozen from the DAG so readiness is a cheap local check.
-- ==================================================
DEFINE TABLE OVERWRITE _00_step_run SCHEMAFULL PERMISSIONS NONE;

DEFINE FIELD OVERWRITE workflow_run ON TABLE _00_step_run TYPE record<_00_workflow_run>;
DEFINE FIELD OVERWRITE step ON TABLE _00_step_run TYPE string;
DEFINE FIELD OVERWRITE depends_on ON TABLE _00_step_run TYPE array<string> DEFAULT [];
DEFINE FIELD OVERWRITE status ON TABLE _00_step_run TYPE string DEFAULT 'blocked'
    ASSERT $value INSIDE ['blocked', 'ready', 'dispatched', 'success', 'failed', 'skipped'];
DEFINE FIELD OVERWRITE job_id ON TABLE _00_step_run TYPE option<string>;
-- The step job's captured `result`, handed to dependent steps' payloads.
-- `any` rather than `option<any>`: `any` already admits NONE.
DEFINE FIELD OVERWRITE output ON TABLE _00_step_run TYPE any;
DEFINE FIELD OVERWRITE error ON TABLE _00_step_run TYPE option<object> FLEXIBLE;
DEFINE FIELD OVERWRITE created_at ON TABLE _00_step_run TYPE datetime DEFAULT time::now() READONLY;
DEFINE FIELD OVERWRITE updated_at ON TABLE _00_step_run TYPE datetime VALUE time::now();
DEFINE FIELD OVERWRITE finished_at ON TABLE _00_step_run TYPE option<datetime>;

DEFINE INDEX OVERWRITE idx_step_wf ON TABLE _00_step_run COLUMNS workflow_run, step UNIQUE;
DEFINE INDEX OVERWRITE idx_step_job ON TABLE _00_step_run COLUMNS job_id;
DEFINE INDEX OVERWRITE idx_step_status ON TABLE _00_step_run COLUMNS workflow_run, status;
