import { RecordId, SchemaStructure } from '@spooky/query-builder';
import { Level } from 'pino';
import { PushEventOptions } from './events/index';
import { UpEvent } from './modules/sync/index';

export type { Level } from 'pino';

/**
 * The type of storage backend to use for the local database.
 * - 'memory': In-memory storage (transient).
 * - 'indexeddb': IndexedDB storage (persistent).
 */
export type StoreType = 'memory' | 'indexeddb';

/**
 * Interface for a custom persistence client.
 * Allows providing a custom storage mechanism for the local database.
 */
export interface PersistenceClient {
  /**
   * Sets a value in the storage.
   * @param key The key to set.
   * @param value The value to store.
   */
  set<T>(key: string, value: T): Promise<void>;
  /**
   * Gets a value from the storage.
   * @param key The key to retrieve.
   * @returns The stored value or null if not found.
   */
  get<T>(key: string): Promise<T | null>;
  /**
   * Removes a value from the storage.
   * @param key The key to remove.
   */
  remove(key: string): Promise<void>;
}

/**
 * Supported Time-To-Live (TTL) values for cached queries.
 * Format: number + unit (m=minutes, h=hours, d=days).
 */
export type QueryTimeToLive =
  | '1m'
  | '5m'
  | '10m'
  | '15m'
  | '20m'
  | '25m'
  | '30m'
  | '1h'
  | '2h'
  | '3h'
  | '4h'
  | '5h'
  | '6h'
  | '7h'
  | '8h'
  | '9h'
  | '10h'
  | '11h'
  | '12h'
  | '1d';

/**
 * Result object returned when a query is registered or executed.
 */
export interface SpookyQueryResult {
  /** The unique hash identifier for the query. */
  hash: string;
}

export type SpookyQueryResultPromise = Promise<SpookyQueryResult>;

export interface EventSubscriptionOptions {
  priority?: number;
}

/**
 * Configuration options for the Spooky client.
 * @template S The schema structure type.
 */
export interface SpookyConfig<S extends SchemaStructure> {
  /** Database connection configuration. */
  database: {
    /** The SurrealDB endpoint URL. */
    endpoint?: string;
    /** The namespace to use. */
    namespace: string;
    /** The database name. */
    database: string;
    /** The local store type implementation. */
    store?: StoreType;
    /** Authentication token. */
    token?: string;
  };
  /** Unique client identifier. If not provided, one will be generated. */
  clientId?: string;
  /** The schema definition. */
  schema: S;
  /** The compiled SURQL schema string. */
  schemaSurql: string;
  /** Logging level. */
  logLevel: Level;
  /**
   * Persistence client to use.
   * Can be a custom implementation, 'surrealdb' (default), or 'localstorage'.
   */
  persistenceClient?: PersistenceClient | 'surrealdb' | 'localstorage';
  /** OpenTelemetry collector endpoint for telemetry data. */
  otelEndpoint?: string;
  /**
   * Debounce time in milliseconds for stream updates.
   * Defaults to 100ms.
   */
  streamDebounceTime?: number;
}

export type QueryHash = string;

// Flat array format: [[record-id, version], [record-id, version], ...]
export type RecordVersionArray = Array<[string, number]>;

/**
 * Represents the difference between two record version sets.
 * Used for synchronizing local and remote states.
 */
export interface RecordVersionDiff {
  /** List of records added. */
  added: Array<{ id: RecordId<string>; version: number }>;
  /** List of records updated. */
  updated: Array<{ id: RecordId<string>; version: number }>;
  /** List of record IDs removed. */
  removed: RecordId<string>[];
}

/**
 * Configuration for a specific query instance.
 * Stores metadata about the query's state, parameters, and versioning.
 */
export interface QueryConfig {
  /** The unique ID of the query config record. */
  id: RecordId<string>;
  /** The SURQL query string. */
  surql: string;
  /** Parameters used in the query. */
  params: Record<string, any>;
  /** The version array representing the local state of results. */
  localArray: RecordVersionArray;
  /** The version array representing the remote (server) state of results. */
  remoteArray: RecordVersionArray;
  /** Time-To-Live for this query. */
  ttl: QueryTimeToLive;
  /** Timestamp when the query was last accessed/active. */
  lastActiveAt: Date;
  /** The name of the table this query targets (if applicable). */
  tableName: string;
}

export type QueryConfigRecord = QueryConfig & { id: string };

/**
 * Internal state of a live query.
 */
export interface QueryState {
  /** The configuration for this query. */
  config: QueryConfig;
  /** The current cached records for this query. */
  records: Record<string, any>[];
  /** Timer for TTL expiration. */
  ttlTimer: NodeJS.Timeout | null;
  /** TTL duration in milliseconds. */
  ttlDurationMs: number;
  /** Number of times the query has been updated. */
  updateCount: number;
}

// Callback types
export type QueryUpdateCallback = (records: Record<string, any>[]) => void;
export type MutationCallback = (mutations: UpEvent[]) => void;

export type MutationEventType = 'create' | 'update' | 'delete';

// Mutation event for sync
/**
 * Represents a mutation event (create, update, delete) to be synchronized.
 */
export interface MutationEvent {
  /** Example: 'create', 'update', or 'delete'. */
  type: MutationEventType;
  /** unique id of the mutation */
  mutation_id: RecordId<string>;
  /** The ID of the record being mutated. */
  record_id: RecordId<string>;
  /** The data payload for create/update operations. */
  data?: any;
  /** The full record data (optional context). */
  record?: any;
  /** Options for the mutation event (e.g., debounce settings). */
  options?: PushEventOptions;
  /** Timestamp when the event was created. */
  createdAt: Date;
}

/**
 * Options for run operations.
 */
export interface RunOptions {
  assignedTo?: string;
  max_retries?: number;
  retry_strategy?: 'linear' | 'exponential';
}

/**
 * Options for update operations.
 */
export interface UpdateOptions {
  /**
   * Debounce configuration for the update.
   * If boolean, enables default debounce behavior.
   */
  debounced?: boolean | DebounceOptions;
}

/**
 * Configuration options for debouncing updates.
 */
export interface DebounceOptions {
  /**
   * The key to use for debouncing.
   * - 'recordId': Debounce based on the specific record ID. WARNING: IT WILL ONLY ACCEPT THE LATEST CHANGE AND DOES *NOT* MERGE THE PREVIOUS ONCES. IF YOU ARE UNSURE JUST USE 'recordId_x_fields'.
   * - 'recordId_x_fields': Debounce based on record ID and specific fields.
   */
  key?: 'recordId' | 'recordId_x_fields';
  /** The debounce delay in milliseconds. */
  delay?: number;
}
