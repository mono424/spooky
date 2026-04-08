import type { SyncedDbConfig } from '@spooky-sync/client-solid';
import { createOtelTransmit } from '@spooky-sync/core/otel';
import { schema, SURQL_SCHEMA } from './schema.gen';

// Database configuration
export const dbConfig: SyncedDbConfig<typeof schema> = {
  logLevel: 'trace',
  otelTransmit: createOtelTransmit('/v1/logs'),
  schema: schema,
  schemaSurql: SURQL_SCHEMA,
  database: {
    namespace: 'main',
    database: 'example',
    endpoint: 'ws://localhost:8666/rpc',
    store: 'memory',
    persistenceClient: 'localstorage',
  },
};
