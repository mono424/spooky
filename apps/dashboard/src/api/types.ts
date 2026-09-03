/** Wire types for the scheduler's `/admin/api` surface. */

export type SessionMode = 'roster' | 'breakglass' | 'mcp';

/** What a session may do. Roster and break-glass sessions are always `full`. */
export type SessionScope = 'read' | 'full';

export interface ServerConfig {
  scheduler_id: string;
  version: string;
  /** Whether `SPKY_ADMIN_PASSWORD` is set, i.e. password-only login is offered. */
  breakglass_available: boolean;
  /**
   * Whether this scheduler can reach the Sp00ky Cloud control plane. Off for a
   * self-hosted scheduler, and for a cloud tenant that has not been restarted
   * since the link was introduced. Cloud-only actions are offered disabled,
   * with that reason, rather than hidden.
   */
  cloud_linked?: boolean;
  /** The project slug this scheduler serves; names the MCP server entry. */
  project_slug?: string;
  /** Whether tokens outlive the process (true when `SPKY_AUTH_SECRET` is set). */
  sessions_persistent?: boolean;
  /**
   * Whether something will relaunch the process after it exits. A scheduler
   * run from a checkout is not supervised, and a restart from the dashboard
   * would simply stop it.
   */
  supervised?: boolean;
}

export interface LoginResponse {
  token: string;
  mode: SessionMode;
  subject: string;
  label: string;
  expires_in_secs: number;
  scope?: SessionScope;
}

export interface MeResponse {
  subject: string;
  label: string;
  mode: SessionMode;
  scope?: SessionScope;
}

/** `POST /admin/api/tokens`: a long-lived MCP token, shown exactly once. */
export interface TokenResponse {
  token: string;
  label: string;
  scope: SessionScope;
  expires_at: string;
  /** Path of the MCP endpoint on this scheduler, e.g. `/admin/api/mcp`. */
  endpoint: string;
}

/** One entry of the MCP server's `tools/list`. */
export interface McpTool {
  name: string;
  description: string;
  inputSchema: unknown;
  annotations?: {
    title?: string;
    readOnlyHint?: boolean;
    destructiveHint?: boolean;
  };
}

/** One end-to-end heartbeat probe cycle. `ms` is null for a failed cycle. */
export interface HeartbeatSample {
  ts: number;
  ms: number | null;
  ok: boolean;
}

export interface HeartbeatInfo {
  enabled?: boolean;
  last_e2e_ms?: number | null;
  last_ok_epoch_ms?: number | null;
  last_attempt_epoch_ms?: number | null;
  consecutive_failures?: number;
  stale?: boolean;
  blocked_reason?: string | null;
  samples?: HeartbeatSample[];
}

export type SchedulerStatus =
  | 'cloning'
  | 'ready'
  | 'frozen'
  | 'updating'
  | 'restoring';

export interface SchedulerEntity {
  entity: 'scheduler';
  id: string;
  ip: string | null;
  status: SchedulerStatus;
  views: number;
  version: string;
  surrealdb_version: string;
  uptime_seconds: number;
  pending_events: number;
  snapshot_seq: number;
  latest_seq: number;
  lag: number;
  heartbeat: HeartbeatInfo;
  env: Record<string, string>;
}

export type SspStatus = 'bootstrapping' | 'replaying' | 'ready' | 'unknown';

/** Populated only while an SSP is loading its circuit. */
export interface BootstrapProgress {
  tables_done: number;
  tables_total: number;
  rows_loaded: number;
  current_table?: string | null;
}

export interface SspEntity {
  entity: 'ssp';
  id: string;
  ip: string | null;
  status: SspStatus;
  views: number;
  version: string;
  uptime_seconds: number;
  last_heartbeat_seconds_ago: number;
  /** Seconds in the current phase. Null when the scheduler has no record. */
  state_seconds: number | null;
  buffered_events: number;
  bootstrap: BootstrapProgress | null;
  env: Record<string, string> | null;
}

export type BackendStatus = 'healthy' | 'unhealthy' | 'unreachable' | 'unknown';

export interface BackendSummary {
  name: string;
  url: string;
  ip: string | null;
  port: number | null;
  healthcheck: string;
  healthcheck_url: string;
  status: BackendStatus;
  response_time_ms: number | null;
  last_checked: string | null;
  last_healthy: string | null;
}

export interface HealthSample {
  at: number;
  ms: number;
  status: BackendStatus;
  ok: boolean;
}

export interface BackendDetail extends BackendSummary {
  history: HealthSample[];
  env: Record<string, string> | null;
  check_interval_secs: number;
  logs_available: boolean;
}

export interface Overview {
  scheduler: SchedulerEntity | null;
  ssps: SspEntity[];
  backends: BackendSummary[];
  totals: {
    ssps: number;
    ssps_ready: number;
    backends: number;
    backends_healthy: number;
  };
  bootstrap_timeout_secs: number;
  server_time_ms: number;
  /** Operations still running. Finished ones live on `/operations`. */
  operations?: Operation[];
}

/* ------------------------------------------------------------------ */
/* Operations: the scheduler's log of operator actions in flight.       */
/* ------------------------------------------------------------------ */

export type OpKind =
  | 'ssp_restart'
  | 'ssp_clean'
  | 'ssp_reload'
  | 'rolling_restart'
  | 'scheduler_restart'
  | 'reclone'
  | 'rehash'
  | 'cloud_restart'
  | 'backup_create'
  | 'backup_restore';

export type OpStatus = 'running' | 'done' | 'failed';

export interface Operation {
  id: string;
  kind: OpKind;
  /** SSP id, run id, backup id: whatever the action was aimed at. */
  target: string | null;
  requested_by: string;
  /** Epoch ms. */
  started_at: number;
  finished_at: number | null;
  status: OpStatus;
  message: string | null;
  /**
   * Free-form progress. Known shapes: rolling_restart `{done, total, current}`,
   * ssp_restart `{ssp_version}`, backup_create `{backup_id, status, size_bytes}`,
   * backup_restore `{restore_id, stage}`.
   */
  detail: Record<string, unknown>;
}

/* ------------------------------------------------------------------ */
/* Actions                                                              */
/* ------------------------------------------------------------------ */

export type SspRestartMode = 'restart' | 'clean' | 'reload';
export type SchedulerRestartMode = 'restart' | 'reclone' | 'rehash';

export interface CloudRestartRequest {
  roles?: string[];
  upgrade: boolean;
  clean: boolean;
  surreal: boolean;
}

export interface OperationResponse {
  operation: Operation;
}

export interface CancelResponse {
  run: string;
  status: 'killed' | 'kill_requested';
}

export interface RerunResponse {
  run: string;
  rerun_of: string;
}

export interface RetryResponse {
  run: string;
  retry_count: number;
  reset: string[];
  kept: string[];
}

export interface JobKillResponse {
  id: string;
  dispatched: number;
  ssps: number;
}

export interface JobRetryResponse {
  id: string;
  status: string;
  assigned_to: string;
}

/* ------------------------------------------------------------------ */
/* Backups                                                              */
/* ------------------------------------------------------------------ */

export type BackupJobStatus = 'queued' | 'running' | 'completed' | 'failed';

/** The scheduler's own record of a backup it executed. In memory only. */
export interface BackupJobState {
  backup_id: string;
  project_slug: string;
  status: BackupJobStatus;
  enqueued_at: string;
  started_at: string | null;
  finished_at: string | null;
  /** Compressed size of the export. */
  size_bytes: number | null;
  snapshot_seq: number | null;
  storage_path: string | null;
  error: string | null;
}

/**
 * The scheduler's own record of a restore. The three booleans are the real
 * stages; `replica_restored` is the wire name for host state, kept for the
 * control plane which reads it.
 */
export interface RestoreJobState {
  restore_id: string;
  backup_id: string;
  project_slug: string;
  storage_path: string;
  status: BackupJobStatus;
  enqueued_at: string;
  started_at: string | null;
  finished_at: string | null;
  snapshot_seq: number | null;
  pending_cleared: number | null;
  main_db_restored: boolean;
  replica_restored: boolean;
  ssps_evicted: number | null;
  error: string | null;
}

export type CatalogStatus =
  | 'pending'
  | 'in_progress'
  | 'completed'
  | 'failed'
  | 'deleted';

/** One backup as the catalog knows it, joined with the local job if any. */
export interface CatalogEntry {
  id: string;
  name: string | null;
  status: CatalogStatus;
  size_bytes: number;
  storage_path: string | null;
  snapshot_seq: number | null;
  created_at: string;
  completed_at: string | null;
  error: string | null;
  /** `cloud` from the control plane's table, `s3` from a bucket listing. */
  source: 'cloud' | 's3';
  local: BackupJobState | null;
}

export interface BackupConfig {
  enabled: boolean;
  schedule: string | null;
  retention: number | null;
  next_run_at: string | null;
  last_scheduled_at: string | null;
}

export interface BackupsData {
  linked: boolean;
  s3: { configured: boolean; endpoint: string | null; bucket: string | null };
  project_slug: string;
  scheduler_status: string;
  local: {
    current_running: BackupJobState | null;
    queue_len: number;
    recent: BackupJobState[];
  };
  catalog: CatalogEntry[];
  restores: RestoreJobState[];
  /** Null when not linked: schedules are run by Sp00ky Cloud. */
  config: BackupConfig | null;
}

export type RestoreStage =
  | 'queued'
  | 'running'
  | 'main_db'
  | 'replica'
  | 'done'
  | 'failed';

export interface RestoreStatus {
  cloud: {
    id: string;
    status: string;
    created_at: string;
    completed_at: string | null;
    error: string | null;
  } | null;
  local: RestoreJobState | null;
  stage: RestoreStage;
}

export type RunStatus =
  | 'running'
  | 'success'
  | 'failed'
  | 'skipped'
  | 'replaced'
  | 'killed';

export interface WorkflowRun {
  id: string;
  workflow_name: string;
  schedule_name: string | null;
  status: RunStatus;
  kill_requested: boolean;
  error: unknown;
  created_at: string;
  updated_at: string | null;
  finished_at: string | null;
  /** `cron` for a scheduled fire, `manual` for an operator rerun. Null on old rows. */
  trigger?: string | null;
  /** Operator retries applied to this run. Each mints new `_r<n>` step job ids. */
  retry_count?: number;
  /** The run this one was rerun from, when it was. */
  rerun_of?: string | null;
}

export interface WorkflowRunDetail extends WorkflowRun {
  dag: unknown;
  input: unknown;
  target_table: string | null;
}

export interface StepRun {
  step: string;
  depends_on: string[];
  status: string;
  job_id: string | null;
  output: unknown;
  error: unknown;
  created_at: string | null;
  finished_at: string | null;
}

export interface Schedule {
  name: string;
  kind: string;
  cron: string | null;
  every_ms: number | null;
  timezone: string | null;
  paused: boolean;
  config_disabled: boolean;
  concurrency: string;
  max_retries: number | null;
  retry_strategy: string | null;
  /** Seconds. */
  timeout: number | null;
  target_table: string | null;
  path: string | null;
  for_each: string | null;
  for_each_key: string | null;
  history_mode: string | null;
  last_run_status: string | null;
  next_fire_at: string | null;
  last_fire_at: string | null;
  last_run_at: string | null;
  created_at: string | null;
  updated_at: string | null;
  last_error: string | null;
}

/** One fire of a schedule. */
export interface ScheduleRun {
  id: string;
  schedule_name: string;
  key: string;
  kind: string;
  status: RunStatus;
  trigger: string | null;
  job_id: string | null;
  /** Record id of the workflow run this fire produced, when it produced one. */
  workflow_run: string | null;
  error: unknown;
  fire_at: string | null;
  created_at: string | null;
  finished_at: string | null;
}

/**
 * Hourly outcome buckets from `_00_run_rollup`. These survive retention, so a
 * tally built from them does not shrink as run rows are pruned.
 */
export interface RunRollup {
  bucket: string;
  success: number;
  failed: number;
  skipped: number;
  replaced: number;
  killed: number;
}

export interface ScheduleDetailData {
  schedule: Schedule;
  runs: ScheduleRun[];
  rollup: RunRollup[];
}

export interface LogLine {
  ts: number;
  level: string;
  target: string;
  message: string;
  fields?: string;
}
