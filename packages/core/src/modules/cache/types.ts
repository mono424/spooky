import type { RecordId, Duration } from 'surrealdb';
import type { QueryTimeToLive } from '../../types';

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
  op: 'CREATE' | 'UPDATE' | 'DELETE';
  record: RecordWithId;
  version: number;
}
