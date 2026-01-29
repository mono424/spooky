import { RecordId, SchemaStructure } from '@spooky/query-builder';
import { Level } from 'pino';

export type { Level } from 'pino';

export type StoreType = 'memory' | 'indexeddb';

export interface PersistenceClient {
  set<T>(key: string, value: T): Promise<void>;
  get<T>(key: string): Promise<T | null>;
  remove(key: string): Promise<void>;
}

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

export interface SpookyQueryResult {
  hash: string;
}

export type SpookyQueryResultPromise = Promise<SpookyQueryResult>;

export interface EventSubscriptionOptions {
  priority?: number;
}

export interface SpookyConfig<S extends SchemaStructure> {
  database: {
    endpoint?: string;
    namespace: string;
    database: string;
    store?: StoreType;
    token?: string;
  };
  clientId?: string;
  schema: S;
  schemaSurql: string;
  logLevel: Level;
  persistenceClient?: PersistenceClient | 'surrealdb' | 'localstorage';
  otelEndpoint?: string;
}

export type QueryHash = string;

// Flat array format: [[record-id, version], [record-id, version], ...]
export type RecordVersionArray = Array<[string, number]>;

export interface RecordVersionDiff {
  added: Array<{ id: RecordId<string>; version: number }>;
  updated: Array<{ id: RecordId<string>; version: number }>;
  removed: RecordId<string>[];
}

export interface QueryConfig {
  id: RecordId<string>;
  surql: string;
  params: Record<string, any>;
  localArray: RecordVersionArray;
  remoteArray: RecordVersionArray;
  ttl: QueryTimeToLive;
  lastActiveAt: Date;
  tableName: string;
}

export type QueryConfigRecord = QueryConfig & { id: string };

export interface QueryState {
  config: QueryConfig;
  records: Record<string, any>[];
  ttlTimer: NodeJS.Timeout | null;
  ttlDurationMs: number;
  updateCount: number;
}

// Callback types
export type QueryUpdateCallback = (records: Record<string, any>[]) => void;
export type MutationCallback = (mutations: MutationEvent[]) => void;

export type MutationEventType = 'create' | 'update' | 'delete';

// Mutation event for sync
export interface MutationEvent {
  type: MutationEventType;
  mutation_id: RecordId<string>;
  record_id: RecordId<string>;
  data?: any;
  record?: any;
}
