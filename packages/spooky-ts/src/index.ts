import {
  ConfigLayer,
  SpookyConfig,
  QueryManagerServiceLayer,
} from "./services/index.js";
import { DatabaseServiceLayer } from "./services/database-wasm.js";
import { Effect, Layer } from "effect";
import { main } from "./spooky.js";
import { SchemaStructure } from "@spooky/query-builder";
import { AuthManagerServiceLayer } from "./services/auth-manager.js";

export function createSpooky<S extends SchemaStructure>(
  config: SpookyConfig<S>
) {
  const configLayer = ConfigLayer<S>(config);
  const databaseServiceLayer = DatabaseServiceLayer<S>().pipe(
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
    authManagerServiceLayer,
    queryManagerServiceLayer
  );

  return main<S>().pipe(Effect.provide(MainLayer), Effect.runPromise);
}
