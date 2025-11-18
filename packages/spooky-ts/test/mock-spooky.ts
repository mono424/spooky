import { SpookyConfig } from "../src/services/index.js";
import { SchemaStructure } from "@spooky/query-builder";
import { dbContext, createMockDatabaseService } from "./mock-database.js";
import { createLogger } from "../src/services/logger.js";
import { createAuthManagerService } from "../src/services/auth-manager.js";
import { createQueryManagerService } from "../src/services/query-manager.js";
import { createMutationManagerService } from "../src/services/mutation-manager.js";
import { runProvision } from "../src/provision.js";
import { createSpookyInstance } from "../src/spooky.js";

export async function createMockSpooky<S extends SchemaStructure>(
  config: SpookyConfig<S>
) {
  const logger = createLogger(config.logLevel);

  // Create mock database service (using local nodes instead of remote)
  const databaseService = await createMockDatabaseService(config, logger);

  // Run provisioning
  await runProvision(
    config.database,
    config.schemaSurql,
    databaseService,
    logger,
    config.provisionOptions
  );

  // Create services
  const authManager = createAuthManagerService(databaseService);
  const queryManager = createQueryManagerService(
    config.schema,
    databaseService,
    logger
  );
  const mutationManager = createMutationManagerService(
    config.schema,
    databaseService,
    queryManager,
    logger
  );

  // Create and return the Spooky instance
  const spooky = await createSpookyInstance(
    config.schema,
    databaseService,
    authManager,
    queryManager,
    mutationManager
  );

  return {
    spooky,
    dbContext,
  };
}
