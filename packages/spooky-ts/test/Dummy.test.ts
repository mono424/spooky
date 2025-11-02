import { describe, expect, it } from "@effect/vitest";
import { createSpooky } from "../src/index.js";
import type { SpookyConfig } from "../src/config.js";
import { testSchema } from "./test.schema.js";
import { Effect, Layer } from "effect";
import { ConfigLayer } from "../src/config.js";
import {
  MockDatabaseServiceLayer,
  createMockDatabaseService,
} from "./mock-database.js";
import { main } from "../src/spooky.js";

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

  it("should create 3 independent nodes with load balancer", async () => {
    const mockDb = await Effect.runPromise(
      createMockDatabaseService("test", "test")
    );

    expect(mockDb.nodes).toHaveLength(3);
    expect(mockDb.nodes[0].id).toBe("node1");
    expect(mockDb.nodes[1].id).toBe("node2");
    expect(mockDb.nodes[2].id).toBe("node3");
    expect(mockDb.loadBalancer).toBeDefined();

    // Verify each node has its own instance
    expect(mockDb.nodes[0].instance).toBeDefined();
    expect(mockDb.nodes[1].instance).toBeDefined();
    expect(mockDb.nodes[2].instance).toBeDefined();

    // Cleanup
    await Effect.runPromise(mockDb.cleanup);
  });

  it("should distribute requests across nodes using round-robin", async () => {
    const mockDb = await Effect.runPromise(
      createMockDatabaseService("test", "test")
    );

    // Get nodes in sequence
    const node1 = mockDb.loadBalancer.getNextNode();
    const node2 = mockDb.loadBalancer.getNextNode();
    const node3 = mockDb.loadBalancer.getNextNode();
    const node4 = mockDb.loadBalancer.getNextNode(); // Should wrap back to node1

    expect(node1.id).toBe("node1");
    expect(node2.id).toBe("node2");
    expect(node3.id).toBe("node3");
    expect(node4.id).toBe("node1");

    // Verify request counts
    expect(node1.requestCount).toBe(2);
    expect(node2.requestCount).toBe(1);
    expect(node3.requestCount).toBe(1);

    // Cleanup
    await Effect.runPromise(mockDb.cleanup);
  });
});
