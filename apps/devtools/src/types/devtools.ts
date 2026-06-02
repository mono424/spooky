// Core DevTools Types - matches the backend DevToolsService structure

export interface BackendDevToolsState {
  eventsHistory: BackendEvent[];
  activeQueries: Record<number, ActiveQuery>;
  auth: BackendAuthState;
  version: string;
  versions?: VersionsState;
  database: DatabaseState;
}

// Frontend-vs-backend versions of the stack components. Unknown/unreachable
// values are reported as the string 'unavailable'.
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

export interface ActiveQuery {
  queryHash: number;
  status: 'initializing' | 'active' | 'updating' | 'destroyed';
  createdAt: number;
  lastUpdate: number;
  updateCount: number;
  dataSize?: number;
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
  tableData: Record<string, Record<string, unknown>[]>;
  schema?: Record<string, string[]>; // table -> column names
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

export type TabType = 'events' | 'queries' | 'database' | 'auth' | 'mcp' | 'versions';

// Shared default so adapters/stores can fall back when versions are absent.
export const DEFAULT_VERSIONS: VersionsState = {
  frontend: { core: 'unavailable', wasm: 'unavailable', surrealdb: 'unavailable' },
  backend: { ssp: 'unavailable', scheduler: 'unavailable', surrealdb: 'unavailable' },
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
