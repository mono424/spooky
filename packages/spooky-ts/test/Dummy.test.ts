import { describe, expect, it } from "@effect/vitest";
import { createSpooky } from "../src/index.js";
import type { SpookyConfig } from "../src/config.js";
import { testSchema } from "./test.schema.js";
import { Effect, Layer } from "effect";
import { ConfigLayer } from "../src/config.js";
import { MockDatabaseServiceLayer } from "./mock-database.js";
import { main } from "../src/spooky.js";
import { Executor, GetTable } from "@spooky/query-builder";

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
    const spooky = await createSpooky(mockConfig);

    expect(spooky).toBeDefined();
    expect(spooky.create).toBeDefined();
    expect(spooky.read).toBeDefined();
    expect(spooky.update).toBeDefined();
    expect(spooky.delete).toBeDefined();
    expect(spooky.useQuery).toBeDefined();
  });

  it("should initialize with memory storage strategy", async () => {
    const config = { ...mockConfig, storageStrategy: "memory" as const };
    const spooky = await createSpooky(config);
    expect(spooky).toBeDefined();
  });

  it("should initialize with indexeddb storage strategy", async () => {
    const config = { ...mockConfig, storageStrategy: "indexeddb" as const };
    const spooky = await createSpooky(config);
    expect(spooky).toBeDefined();
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

  it("should initialize with 3 mock database nodes", async () => {
    const configLayer = ConfigLayer(mockConfig);
    const mockDatabaseServiceLayer = MockDatabaseServiceLayer<
      typeof testSchema
    >().pipe(Layer.provide(configLayer));

    const AllServices = Layer.mergeAll(configLayer, mockDatabaseServiceLayer);

    const spooky = await main<typeof testSchema>().pipe(
      Effect.provide(AllServices),
      Effect.runPromise
    );

    expect(spooky).toBeDefined();
    expect(spooky.create).toBeDefined();
    expect(spooky.read).toBeDefined();
    expect(spooky.update).toBeDefined();
    expect(spooky.delete).toBeDefined();
    expect(spooky.useQuery).toBeDefined();
  });

  it("should create a query", async () => {
    const configLayer = ConfigLayer(mockConfig);
    const mockDatabaseServiceLayer = MockDatabaseServiceLayer<
      typeof testSchema
    >().pipe(Layer.provide(configLayer));

    const AllServices = Layer.mergeAll(configLayer, mockDatabaseServiceLayer);

    const spooky = await main<typeof testSchema>().pipe(
      Effect.provide(AllServices),
      Effect.runPromise
    );

    // Mock functions to track calls
    let executorCalled = false;
    let cleanupCalled = false;
    let queryString = "";

    const executor: Executor<GetTable<typeof testSchema, "thread">> = (
      query
    ) => {
      executorCalled = true;
      queryString = query.selectQuery().query;
      return {
        cleanup: () => {
          cleanupCalled = true;
        },
      };
    };

    const query = Effect.runSync(
      spooky.useQuery("thread", executor, {})
    ).orderBy("created_at", "asc");

    const result = query.build();

    await result.select();

    expect(queryString).toBe("SELECT * FROM thread ORDER BY created_at asc;");
    expect(executorCalled).toBe(true);
    expect(cleanupCalled).toBe(true);
  });
});
