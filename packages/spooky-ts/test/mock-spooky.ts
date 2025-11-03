import {
  ConfigLayer,
  SpookyConfig,
  QueryManagerServiceLayer,
  AuthManagerServiceLayer,
} from "../src/services/index.js";
import { Effect, Layer } from "effect";
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

  const MainLayer = Layer.mergeAll(
    configLayer,
    databaseServiceLayer,
    queryManagerServiceLayer,
    authManagerServiceLayer
  );

  return {
    spooky: await main<S>().pipe(Effect.provide(MainLayer), Effect.runPromise),
    dbContext,
  };
}
