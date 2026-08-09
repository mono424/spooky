/**
 * Backend versions of the stack components, derived from the entity list the
 * backend `/info` endpoint exposes (read via the `fn::spooky::info()` SurrealQL
 * function). Any component that isn't reported degrades to `'unavailable'`.
 */
export interface BackendVersions {
  ssp: string;
  scheduler: string;
  surrealdb: string;
}

export const UNAVAILABLE = 'unavailable';

/**
 * A single stack entity as reported by `/info` (one per ssp / scheduler /
 * backend). Carries far more than versions — status, uptime, ip, views — so the
 * DevTools can render the whole stack. Extra fields are preserved verbatim.
 */
/** One end-to-end sync-pipeline probe cycle recorded by the scheduler. */
export interface HeartbeatSample {
  ts: number;
  /** `null` for a failed cycle. */
  ms: number | null;
  ok: boolean;
}

/**
 * E2E heartbeat state, reported on the scheduler entity only. The scheduler
 * times a probe row's full round trip (DB event → ingest → broadcast → SSP
 * circuit step); `samples` is its rolling window of recent cycles, which the
 * DevTools sparkline renders.
 */
export interface HeartbeatInfo {
  enabled: boolean;
  stale: boolean;
  /** Nothing to measure right now (e.g. no ready SSPs mid-bootstrap). */
  blocked?: boolean;
  blocked_reason?: string | null;
  last_e2e_ms: number | null;
  last_ok_epoch_ms: number | null;
  consecutive_failures: number;
  interval_secs: number;
  samples?: HeartbeatSample[];
}

export interface BackendEntity {
  entity: string;
  id?: string;
  ip?: string | null;
  status?: string;
  version?: string;
  surrealdb_version?: string;
  uptime_seconds?: number;
  views?: number;
  /** Scheduler entity only. */
  heartbeat?: HeartbeatInfo;
  [key: string]: unknown;
}

export interface BackendInfo {
  versions: BackendVersions;
  entities: BackendEntity[];
}

export function emptyBackendVersions(): BackendVersions {
  return { ssp: UNAVAILABLE, scheduler: UNAVAILABLE, surrealdb: UNAVAILABLE };
}

export function emptyBackendInfo(): BackendInfo {
  return { versions: emptyBackendVersions(), entities: [] };
}

/** Strip a leading `surrealdb-` so versions read as bare semver (e.g. `2.0.3`). */
function normalizeServerVersion(v: string): string {
  return String(v).replace(/^surrealdb-/i, '').trim();
}

/**
 * Normalize whatever `RETURN fn::spooky::info()` resolves to into the entity
 * array. The SurrealQL function returns the parsed `/info` array; depending on
 * how the result is unwrapped it may arrive as the array itself, a single
 * object, or `null`. Tolerant of all three.
 */
export function toEntityArray(raw: unknown): BackendEntity[] {
  if (Array.isArray(raw)) return raw.filter((e): e is BackendEntity => !!e && typeof e === 'object');
  if (raw && typeof raw === 'object') return [raw as BackendEntity];
  return [];
}

/**
 * Derive component versions + the full entity list from a `/info` entity array.
 * `surrealdb` is taken from whichever entity reports `surrealdb_version` (ssp or
 * scheduler). Never throws; missing pieces stay `'unavailable'`.
 */
export function parseBackendInfo(raw: unknown): BackendInfo {
  const entities = toEntityArray(raw);
  const versions = emptyBackendVersions();

  for (const entity of entities) {
    const version = entity.version ? String(entity.version) : undefined;
    if (entity.entity === 'ssp' && version) versions.ssp = version;
    else if (entity.entity === 'scheduler' && version) versions.scheduler = version;

    if (versions.surrealdb === UNAVAILABLE && entity.surrealdb_version) {
      versions.surrealdb = normalizeServerVersion(String(entity.surrealdb_version));
    }
  }

  return { versions, entities };
}
