import { RecordId } from "surrealdb";

export interface EventSubscriptionOptions {
  priority?: number;
}

export interface SpookyConfig {
  database: {
    endpoint?: string;
    namespace: string;
    database: string;
    token?: string;
  };
}

export type QueryHash = number;

export interface Incantation {
  id: QueryHash;
  surrealql: string;
  hash: number;
  lastActiveAt: number;
}
