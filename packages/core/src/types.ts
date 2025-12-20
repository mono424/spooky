import { SchemaStructure } from "@spooky/query-builder";

export interface EventSubscriptionOptions {
  priority?: number;
}

export interface SpookyConfig<S extends SchemaStructure> {
  database: {
    endpoint?: string;
    namespace: string;
    database: string;
    token?: string;
  };
  schema: S;
  schemaSurql: string;
}

export type QueryHash = number;

export interface Incantation {
  id: QueryHash;
  surrealql: string;
  hash: number;
  lastActiveAt: number;
}
