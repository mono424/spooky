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
  localArray: RecordVersionArray;
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
  added: Array<{ id: RecordId<string>; version: number }>;
  updated: Array<{ id: RecordId<string>; version: number }>;
  removed: RecordId<string>[];
}
