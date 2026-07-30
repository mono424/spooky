// Core DevTools Types - matches the backend DevToolsService structure

export interface BackendDevToolsState {
  eventsHistory: BackendEvent[];
  activeQueries: Record<number, ActiveQuery>;
  auth: BackendAuthState;
  version: string;
  versions?: VersionsState;
  database: DatabaseState;
}

// A single stack entity as reported by the backend `/info` (via
// `fn::spooky::info()`): one per ssp / scheduler / backend.
export interface BackendEntity {
  entity: string;
  id?: string;
  ip?: string | null;
  status?: string;
  version?: string;
  surrealdb_version?: string;
  uptime_seconds?: number;
  views?: number;
  [key: string]: unknown;
}

// Frontend-vs-backend versions of the stack components. Unknown/unreachable
// values are reported as the string 'unavailable'. `entities` carries the full
// per-entity stack info for display.
export interface VersionsState {
  frontend: {
    core: string;
    wasm: string;
    surrealdb: string;
  };
  backend: {
    ssp: string;
    scheduler: string;
    surrealdb: string;
  };
  entities: BackendEntity[];
}

export interface BackendEvent {
  id: number;
  timestamp: number;
  eventType: string;
  payload: any;
}

export interface BackendAuthState {
  authenticated: boolean;
  userId?: string;
  timestamp?: number;
}

// Frontend DevTools State - normalized for UI
export interface DevToolsState {
  events: Sp00kyEvent[];
  activeQueries: ActiveQuery[];
  auth: AuthState;
  database: DatabaseState;
  versions: VersionsState;
}

export interface Sp00kyEvent {
  type: string;
  timestamp: number;
  data: unknown;
}

// Per-query processing-time breakdown (mirrors core's QueryTimings). Each phase
// is a rolling-window summary; `registration` is a one-shot per query.
export interface PhaseStat {
  lastMs: number | null;
  p50: number | null;
  p90: number | null;
  p99: number | null;
  count: number;
}

export interface RegistrationTimings {
  parseMs: number | null;
  planMs: number | null;
  snapshotMs: number | null;
  wallMs: number | null;
}

export interface QueryTimings {
  ssp: PhaseStat;
  sspStoreApply: PhaseStat;
  sspCircuitStep: PhaseStat;
  sspTransform: PhaseStat;
  localFetch: PhaseStat;
  remoteFetch: PhaseStat;
  frontend: PhaseStat;
  registration: RegistrationTimings;
  updateCount: number;
  errorCount: number;
}

export interface ActiveQuery {
  queryHash: number;
  status: 'initializing' | 'active' | 'updating' | 'destroyed';
  createdAt: number;
  lastUpdate: number;
  updateCount: number;
  dataSize?: number;
  /** Query cache Time-To-Live (e.g. '10m'), if known. */
  ttl?: string;
  query?: string;
  variables?: Record<string, unknown>;
  listenerCount?: number;
  connectedQueries?: number[];
  dataHash?: number;
  data?: any;
  localHash?: string;
  localArray?: any;
  remoteHash?: string;
  remoteArray?: any;
  /** Detailed per-phase processing-time breakdown (DevTools/MCP debugging). */
  timings?: QueryTimings;
}

// A single point on the Queries-tab timeline strip. Accumulated panel-side
// (the backend only reports the latest `lastUpdate` per query, not a history).
export interface QueryMark {
  queryHash: number;
  timestamp: number;
  kind: 'registered' | 'updated';
}

export interface AuthState {
  isAuthenticated: boolean;
  user: {
    email?: string;
    roles?: string[];
  } | null;
  lastAuthCheck: number;
}

export interface DatabaseState {
  tables: string[];
  /** Tables that exist on the remote DB (enumerated on demand). */
  remoteTables?: string[];
  tableData: Record<string, Record<string, unknown>[]>;
  schema?: Record<string, string[]>; // table -> column names
  /** Durability of the local store. `fallback: true` means persistence was
   *  requested but the dataset is actually only in RAM. */
  storage?: {
    status: 'unknown' | 'persistent' | 'memory';
    fallback: boolean;
    error?: string;
  };
}

// Chrome Extension Message Types

export interface ChromeMessage {
  type: string;
  data?: unknown;
  state?: DevToolsState;
  tableName?: string;
}

export interface Sp00kyTableDataResponse {
  type: 'SP00KY_TABLE_DATA_RESPONSE';
  source: 'sp00ky-devtools-page';
  tableName: string;
  data: Record<string, unknown>[];
}

// UI State Types

export type TabType = 'events' | 'queries' | 'database' | 'auth' | 'mcp' | 'versions' | 'timing';

// Shared default so adapters/stores can fall back when versions are absent.
export const DEFAULT_VERSIONS: VersionsState = {
  frontend: { core: 'unavailable', wasm: 'unavailable', surrealdb: 'unavailable' },
  backend: { ssp: 'unavailable', scheduler: 'unavailable', surrealdb: 'unavailable' },
  entities: [],
};

export interface UIState {
  activeTab: TabType;
  selectedQueryHash: number | null;
  selectedTable: string | null;
  theme: 'light' | 'dark' | 'auto';
}

// Utility Types

export type EvalResult<T> = T | { error: string };

export interface TableColumn {
  key: string;
  label: string;
  type?: string;
}
