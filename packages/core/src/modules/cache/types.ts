import { RecordId, Duration } from 'surrealdb';
import { QueryTimeToLive, RecordVersionArray } from '../../types.js';

export interface QueryConfig {
  id: RecordId<string>;
  sql: string;
  params: Record<string, any>;
  ttl: QueryTimeToLive | Duration;
  lastActiveAt: Date;
}

export interface CacheRecord {
  table: string;
  op: string;
  id: string;
  record: any;
  version?: number;
}
