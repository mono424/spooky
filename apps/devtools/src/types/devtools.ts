// Core DevTools Types - matches the backend DevToolsService structure

export interface BackendDevToolsState {
  eventsHistory: BackendEvent[];
  activeQueries: Record<number, ActiveQuery>;
  auth: BackendAuthState;
  version: string;
  database: DatabaseState;
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
  events: SpookyEvent[];
  activeQueries: ActiveQuery[];
  auth: AuthState;
  database: DatabaseState;
}

export interface SpookyEvent {
  type: string;
  timestamp: number;
  data: unknown;
}

export interface ActiveQuery {
  queryHash: number;
  status: "initializing" | "active" | "updating" | "destroyed";
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
}

// Chrome Extension Message Types

export interface ChromeMessage {
  type: string;
  data?: unknown;
  state?: DevToolsState;
  tableName?: string;
}

export interface SpookyTableDataResponse {
  type: "SPOOKY_TABLE_DATA_RESPONSE";
  source: "spooky-devtools-page";
  tableName: string;
  data: Record<string, unknown>[];
}

// UI State Types

export type TabType = "events" | "queries" | "database" | "auth";

export interface UIState {
  activeTab: TabType;
  selectedQueryHash: number | null;
  selectedTable: string | null;
  theme: "light" | "dark" | "auto";
}

// Utility Types

export type EvalResult<T> = T | { error: string };

export interface TableColumn {
  key: string;
  label: string;
  type?: string;
}
