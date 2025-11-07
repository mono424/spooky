import {
  ConfigLayer,
  SpookyConfig,
  QueryManagerServiceLayer,
  AuthManagerServiceLayer,
  MutationManagerServiceLayer,
} from "../src/services/index.js";
import { DevTools } from "@effect/experimental";
import { Effect, Layer, Logger, LogLevel } from "effect";
import { main } from "../src/spooky.js";
import { SchemaStructure } from "@spooky/query-builder";
import { dbContext, MockDatabaseServiceLayer } from "./mock-database.js";

export async function createMockSpooky<S extends SchemaStructure>(
  config: SpookyConfig<S>
) {
  const configLayer = ConfigLayer<S>(config);
  const databaseServiceLayer = MockDatabaseServiceLayer<S>().pipe(
    Layer.provide(configLayer)
  );
  const authManagerServiceLayer = AuthManagerServiceLayer<S>().pipe(
    Layer.provide(databaseServiceLayer)
  );
  const queryManagerServiceLayer = QueryManagerServiceLayer<S>().pipe(
    Layer.provide(configLayer),
    Layer.provide(databaseServiceLayer),
    Layer.provide(authManagerServiceLayer)
  );

  const mutationManagerServiceLayer = MutationManagerServiceLayer<S>().pipe(
    Layer.provide(configLayer),
    Layer.provide(databaseServiceLayer),
    Layer.provide(authManagerServiceLayer),
    Layer.provide(queryManagerServiceLayer)
  );

  const DevToolsLive = DevTools.layer();
  const logger = Logger.minimumLogLevel(LogLevel.Debug);

  const MainLayer = Layer.mergeAll(
    configLayer,
    databaseServiceLayer,
    queryManagerServiceLayer,
    authManagerServiceLayer,
    mutationManagerServiceLayer,
    DevToolsLive,
    logger
  );

  return {
    spooky: await main<S>().pipe(Effect.provide(MainLayer), Effect.runPromise),
    dbContext,
  };
}
