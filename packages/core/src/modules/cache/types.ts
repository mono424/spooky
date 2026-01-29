import { RecordId, Duration } from 'surrealdb';
import { QueryTimeToLive, RecordVersionArray } from '../../types.js';

export type RecordWithId = Record<string, any> & { id: RecordId<string> };

export interface QueryConfig {
  queryHash: string;
  surql: string;
  params: Record<string, any>;
  ttl: QueryTimeToLive | Duration;
  lastActiveAt: Date;
}

export interface CacheRecord {
  table: string;
  op: string;
  record: RecordWithId;
  version?: number;
}
