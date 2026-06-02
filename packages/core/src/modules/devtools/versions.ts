import type { Logger } from '../../services/logger/index';

/**
 * Backend versions of the stack components, as reported over HTTP by the ssp
 * and scheduler services. Any component that can't be reached degrades to
 * `'unavailable'` rather than throwing.
 */
export interface BackendVersions {
  ssp: string;
  scheduler: string;
  surrealdb: string;
}

export const UNAVAILABLE = 'unavailable';

export function emptyBackendVersions(): BackendVersions {
  return { ssp: UNAVAILABLE, scheduler: UNAVAILABLE, surrealdb: UNAVAILABLE };
}

/**
 * Derive the HTTP base URL of the ssp service from the WebSocket endpoint the
 * client connects to. ssp serves its WS upgrade and its `/version` / `/info`
 * HTTP routes on the same host:port, so `ws://h:p` -> `http://h:p` (and
 * `wss` -> `https`). Returns null for missing/invalid endpoints.
 */
export function httpBaseFromWsEndpoint(endpoint: string | undefined): string | null {
  if (!endpoint) return null;
  try {
    const u = new URL(endpoint);
    const proto =
      u.protocol === 'wss:' ? 'https:' : u.protocol === 'ws:' ? 'http:' : u.protocol;
    return `${proto}//${u.host}`; // host includes the port
  } catch {
    return null;
  }
}

async function fetchJson(url: string, timeoutMs = 3000): Promise<any | null> {
  try {
    const res = await fetch(url, { signal: AbortSignal.timeout(timeoutMs) });
    if (!res.ok) return null;
    return await res.json();
  } catch {
    return null;
  }
}

/**
 * Best-effort discovery of backend component versions. Reads ssp `/version`
 * (ssp + wasm-core version) and ssp `/info` (SurrealDB server version +
 * scheduler URL), then the scheduler `/info` for the scheduler version.
 *
 * Never throws: each component that can't be reached stays `'unavailable'`.
 */
export async function fetchBackendVersions(
  endpoint: string | undefined,
  logger?: Logger
): Promise<BackendVersions> {
  const result = emptyBackendVersions();
  const base = httpBaseFromWsEndpoint(endpoint);
  if (!base) {
    logger?.debug(
      { endpoint, Category: 'sp00ky-client::DevToolsService::versions' },
      'No HTTP base derivable from endpoint; backend versions unavailable'
    );
    return result;
  }

  // ssp service version (also the backend wasm-core version: same Rust crate).
  const version = await fetchJson(`${base}/version`);
  if (version?.version) result.ssp = String(version.version);

  // ssp /info: SurrealDB server version + scheduler URL for discovery.
  const info = await fetchJson(`${base}/info`);
  const sspEntity = Array.isArray(info) ? info[0] : undefined;
  if (sspEntity?.surrealdb_version) result.surrealdb = String(sspEntity.surrealdb_version);
  // Fallback: derive ssp version from /info if /version was unreachable.
  if (result.ssp === UNAVAILABLE && sspEntity?.version) result.ssp = String(sspEntity.version);

  const schedulerUrl: string | undefined = sspEntity?.env?.SPKY_SCHEDULER_URL;
  if (schedulerUrl) {
    const sInfo = await fetchJson(`${schedulerUrl.replace(/\/$/, '')}/info`);
    const sched = Array.isArray(sInfo)
      ? sInfo.find((e: any) => e?.entity === 'scheduler')
      : undefined;
    if (sched?.version) result.scheduler = String(sched.version);
  }

  logger?.debug(
    { ...result, Category: 'sp00ky-client::DevToolsService::versions' },
    'Fetched backend versions'
  );
  return result;
}
