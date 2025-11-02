import {
  ConfigLayer,
  SpookyConfig,
  DatabaseServiceLayer,
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

  const MainLayer = Layer.mergeAll(configLayer, databaseServiceLayer);
  return main<S>().pipe(Effect.provide(MainLayer), Effect.runPromise);
}
