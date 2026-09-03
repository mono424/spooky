/** Wire types for the scheduler's `/admin/api` surface. */

export type SessionMode = 'roster' | 'breakglass';

export interface ServerConfig {
  scheduler_id: string;
  version: string;
  /** Whether `SPKY_ADMIN_PASSWORD` is set, i.e. password-only login is offered. */
  breakglass_available: boolean;
}

export interface LoginResponse {
  token: string;
  mode: SessionMode;
  subject: string;
  label: string;
  expires_in_secs: number;
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
