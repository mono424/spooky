import { RecordId, SchemaStructure } from "@spooky/query-builder";

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
  clientId?: string;
  schema: S;
  schemaSurql: string;
}

export type QueryHash = string;

export interface Incantation {
  id: RecordId<QueryHash>;
  surrealql: string;
  hash: string;
  lastActiveAt: number;
}

