import { RecordId, SchemaStructure } from '@spooky/query-builder';
import { Level } from 'pino';

export type { Level } from 'pino';

export type StoreType = 'memory' | 'indexdb';

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
}

export type QueryHash = string;

import { Duration } from 'surrealdb';

export interface Incantation {
  id: RecordId<QueryHash>;
  surrealql: string;
  params?: Record<string, any>;
  localHash: string;
  localArray: RecordVersionArray;
  remoteHash: string;
  remoteArray: RecordVersionArray;
  lastActiveAt: number | Date | string;
  ttl: QueryTimeToLive | Duration;
  meta: {
    tableName: string;
    involvedTables?: string[];
  };
}

// Flat array format: [[record-id, version], [record-id, version], ...]
export type RecordVersionArray = Array<[string, number]>;

export interface RecordVersionDiff {
  added: RecordId<string>[];
  updated: RecordId<string>[];
  removed: RecordId<string>[];
}

// Legacy types - deprecated, kept for backward compatibility during migration
export interface IdTree {
  hash: string;
  children?: Record<string, IdTree>;
  leaves?: { id: string; hash: string }[];
}

export interface IdTreeDiff {
  added: RecordId<string>[];
  updated: RecordId<string>[];
  removed: RecordId<string>[];
}
