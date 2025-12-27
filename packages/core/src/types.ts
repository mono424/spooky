import { RecordId, SchemaStructure } from "@spooky/query-builder";

export type QueryTimeToLive = "1m" | "5m" | "10m" | "15m" | "20m" | "25m" | "30m" | "1h" | "2h" | "3h" | "4h" | "5h" | "6h" | "7h" | "8h" | "9h" | "10h" | "11h" | "12h" | "1d";

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
  ttl: QueryTimeToLive;
  tree: any;
}

