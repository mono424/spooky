import { describe, expect, it } from "@effect/vitest";
import {
  ConfigLayer,
  QueryManagerServiceLayer,
  type SpookyConfig,
} from "../src/services/index.js";
import { testSchema } from "./test.schema.js";
import { Effect, Layer } from "effect";
import { MockDatabaseServiceLayer } from "./mock-database.js";
import { main } from "../src/spooky.js";
import { SchemaStructure } from "@spooky/query-builder";

function createTestSpooky<S extends SchemaStructure>(config: SpookyConfig<S>) {
  const configLayer = ConfigLayer<S>(config);
  const mockDatabaseServiceLayer = MockDatabaseServiceLayer<S>().pipe(
    Layer.provide(configLayer)
  );
  const queryManagerServiceLayer = QueryManagerServiceLayer<S>().pipe(
    Layer.provide(configLayer),
    Layer.provide(mockDatabaseServiceLayer)
  );

  const MainLayer = Layer.mergeAll(
    configLayer,
    mockDatabaseServiceLayer,
    queryManagerServiceLayer
  );

  return main<S>().pipe(Effect.provide(MainLayer), Effect.runPromise);
}

describe("Spooky Initialization", () => {
  const mockConfig: SpookyConfig<typeof testSchema> = {
    schema: testSchema,
    schemaSurql: "DEFINE TABLE test;",
    remoteUrl: "ws://localhost:8000",
    localDbName: "test-local",
    internalDbName: "test-internal",
    storageStrategy: "memory" as const,
    namespace: "test",
    database: "test",
  };

  it("should initialize spooky with valid config", async () => {
    const spooky = await createTestSpooky(mockConfig);

    expect(spooky).toBeDefined();
    expect(spooky.create).toBeDefined();
    expect(spooky.update).toBeDefined();
    expect(spooky.delete).toBeDefined();
    expect(spooky.query).toBeDefined();
  });
});

describe("Mock Database with 3 Nodes", () => {
  const mockConfig: SpookyConfig<typeof testSchema> = {
    schema: testSchema,
    schemaSurql: "DEFINE TABLE test;",
    remoteUrl: "ws://localhost:8000", // This will be ignored
    localDbName: "test-local",
    internalDbName: "test-internal",
    storageStrategy: "memory" as const,
    namespace: "test",
    database: "test",
  };

  it("should create a query", async () => {
    const spooky = await createTestSpooky(mockConfig);

    const query = Effect.runSync(spooky.query("thread", {}));
    query.orderBy("created_at", "asc");

    const result = query.build().select();
    expect(result).toBeDefined();
  });
});
