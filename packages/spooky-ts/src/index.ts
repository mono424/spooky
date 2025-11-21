import { SchemaStructure } from "@spooky/query-builder";
import { createSpookyEventSystem, SpookyConfig } from "./services/index.js";
import { createDatabaseService } from "./services/database-wasm.js";
import { createAuthManagerService } from "./services/auth-manager.js";
import { createQueryManagerService } from "./services/query-manager.js";
import { createMutationManagerService } from "./services/mutation-manager.js";
import { createDevToolsService } from "./services/devtools-service.js";
import { createLogger } from "./services/logger.js";
import { runProvision } from "./provision.js";
import { createSpookyInstance, SpookyInstance } from "./spooky.js";

// Re-export types and values
export * from "./types.js";
export type { SpookyInstance } from "./spooky.js";

// Re-export common query-builder exports as values
export { QueryBuilder, RecordId } from "@spooky/query-builder";
export type {
  GetTable,
  SchemaStructure,
  TableModel,
  TableNames,
} from "@spooky/query-builder";

// Re-export Surreal type and value
export type { Surreal } from "surrealdb";

export async function createSpooky<S extends SchemaStructure>(
  config: SpookyConfig<S>
): Promise<SpookyInstance<S>> {
  const logger = createLogger(config.logLevel);
  const eventSystem = createSpookyEventSystem();

  // Create database service
  const databaseService = await createDatabaseService(config, logger);

  // Run provisioning
  await runProvision(
    config.database,
    config.schemaSurql,
    databaseService,
    logger,
    config.provisionOptions
  );

  // Create services
  const authManager = createAuthManagerService(databaseService, eventSystem);
  const queryManager = createQueryManagerService(
    config.schema,
    databaseService,
    authManager,
    logger,
    eventSystem
  );
  const mutationManager = createMutationManagerService(
    config.schema,
    databaseService,
    queryManager,
    logger
  );

  // Create DevTools service to expose state to Chrome DevTools
  // This service is intentionally not returned in the instance as it works
  // in the background by exposing window.__SPOOKY__ API
  createDevToolsService(eventSystem, logger, {
    version: "0.1.0", // TODO: Get from package.json
    enabled: true,
  });

  // Create and return the Spooky instance
  return createSpookyInstance(
    config.schema,
    databaseService,
    authManager,
    queryManager,
    mutationManager,
    eventSystem
  );
}
