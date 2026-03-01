import { SyncedDbConfig } from '@spooky-sync/client-solid';
import { schema, SURQL_SCHEMA } from './schema.gen';

// Database configuration
export const dbConfig: SyncedDbConfig<typeof schema> = {
  logLevel: 'trace',
  otelEndpoint: '/v1/logs',
  schema: schema,
  schemaSurql: SURQL_SCHEMA,
  database: {
    namespace: 'main',
    database: 'main',
    endpoint: 'ws://localhost:8666/rpc',
    store: 'memory',
    persistenceClient: 'localstorage',
  },
};
