import {
  ConfigLayer,
  SpookyConfig,
  DatabaseServiceLayer,
  QueryManagerServiceLayer,
} from "./services/index.js";
import { Effect, Layer } from "effect";
import { main } from "./spooky.js";
import { SchemaStructure } from "@spooky/query-builder";

export function createSpooky<S extends SchemaStructure>(
  config: SpookyConfig<S>
) {
  const configLayer = ConfigLayer<S>(config);
  const databaseServiceLayer = DatabaseServiceLayer<S>().pipe(
    Layer.provide(configLayer)
  );
  const queryManagerServiceLayer = QueryManagerServiceLayer<S>().pipe(
    Layer.provide(configLayer),
    Layer.provide(databaseServiceLayer)
  );

  const MainLayer = Layer.mergeAll(
    configLayer,
    databaseServiceLayer,
    queryManagerServiceLayer
  );

  return main<S>().pipe(Effect.provide(MainLayer), Effect.runPromise);
}
