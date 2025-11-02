import { Surreal } from "surrealdb";
import { createWasmEngines } from "@surrealdb/wasm";
import { Context, Data, Effect, Layer } from "effect";
import { Config, makeConfig } from "../src/config.js";
import {
  DatabaseService,
  LocalDatabaseError,
  RemoteDatabaseError,
} from "../src/database.js";
import { SchemaStructure } from "@spooky/query-builder";

/**
 * Mock database implementation that uses 3 local SurrealDB WASM nodes
 * instead of a remote database. This simulates a distributed system
 * for testing purposes.
 */

export class MockNodeError extends Data.TaggedError("MockNodeError")<{
  readonly cause?: unknown;
  readonly message: string;
  readonly nodeId: string;
}> {}

interface MockNode {
  id: string;
  instance: Surreal;
  requestCount: number;
}

/**
 * Creates a single mock SurrealDB node using WASM
 */
const createMockNode = (nodeId: string, namespace: string, database: string) =>
  Effect.tryPromise({
    try: async () => {
      const node = new Surreal({
        engines: createWasmEngines({
          capabilities: {
            experimental: {
              allow: ["record_references"],
            },
          },
        }),
        codecOptions: {
          useNativeDates: false,
        },
      });

      // Each node uses its own in-memory database
      await node.connect("mem://");
      await node.use({
        namespace: namespace,
        database: `${database}_${nodeId}`,
      });

      return {
        id: nodeId,
        instance: node,
        requestCount: 0,
      } as MockNode;
    },
    catch: (error) =>
      new MockNodeError({
        message: `Failed to create mock node ${nodeId}`,
        cause: error,
        nodeId,
      }),
  });

/**
 * Load balancer that distributes requests across the 3 nodes
 * using a simple round-robin strategy
 */
class NodeLoadBalancer {
  private currentIndex = 0;

  constructor(private nodes: MockNode[]) {}

  /**
   * Get the next node in round-robin fashion
   */
  getNextNode(): MockNode {
    const node = this.nodes[this.currentIndex];
    node.requestCount++;
    this.currentIndex = (this.currentIndex + 1) % this.nodes.length;
    return node;
  }

  /**
   * Get a specific node by ID
   */
  getNodeById(nodeId: string): MockNode | undefined {
    return this.nodes.find((n) => n.id === nodeId);
  }

  /**
   * Get all nodes
   */
  getAllNodes(): MockNode[] {
    return this.nodes;
  }

  /**
   * Get node with least load
   */
  getLeastLoadedNode(): MockNode {
    return this.nodes.reduce((prev, current) =>
      current.requestCount < prev.requestCount ? current : prev
    );
  }
}

/**
 * Wrapper function for using a mock node
 */
const useMockNode =
  (loadBalancer: NodeLoadBalancer) =>
  <T>(
    fn: (db: Surreal) => Effect.Effect<T, RemoteDatabaseError, never>
  ): Effect.Effect<T, RemoteDatabaseError, never> =>
    Effect.gen(function* () {
      // Get the next node in round-robin fashion
      const node = loadBalancer.getNextNode();

      const result = yield* Effect.try({
        try: () => fn(node.instance),
        catch: (error) =>
          new RemoteDatabaseError({
            message: `Failed to use mock node ${node.id} [sync]`,
            cause: error,
          }),
      });

      if (result instanceof Promise) {
        return yield* Effect.tryPromise({
          try: () => result,
          catch: (error) =>
            new RemoteDatabaseError({
              message: `Failed to use mock node ${node.id} [async]`,
              cause: error,
            }),
        });
      } else {
        return result;
      }
    });

/**
 * Wrapper function for using local database
 */
const useLocalDatabase =
  (db: Surreal) =>
  <T>(
    fn: (db: Surreal) => Effect.Effect<T, LocalDatabaseError, never>
  ): Effect.Effect<T, LocalDatabaseError, never> =>
    Effect.gen(function* () {
      const result = yield* Effect.try({
        try: () => fn(db),
        catch: (error) =>
          new LocalDatabaseError({
            message: "Failed to use database [sync]",
            cause: error,
          }),
      });

      if (result instanceof Promise) {
        return yield* Effect.tryPromise({
          try: () => result,
          catch: (error) =>
            new LocalDatabaseError({
              message: "Failed to use database [async]",
              cause: error,
            }),
        });
      } else {
        return result;
      }
    });

/**
 * Mock Database Service Layer that creates 3 local nodes instead of
 * connecting to a remote database
 */
export const MockDatabaseServiceLayer = <S extends SchemaStructure>() =>
  Layer.scoped(
    DatabaseService,
    Effect.gen(function* () {
      const config = yield* (yield* makeConfig<S>()).getConfig;
      const { localDbName, namespace, database } = config;

      // Create local and internal databases (same as real implementation)
      const internalDatabase = yield* Effect.acquireRelease(
        Effect.tryPromise({
          try: async () => {
            const db = new Surreal({
              engines: createWasmEngines({
                capabilities: {
                  experimental: {
                    allow: ["record_references"],
                  },
                },
              }),
              codecOptions: {
                useNativeDates: false,
              },
            });
            await db.connect("mem://");
            await db.use({
              namespace: "internal",
              database: "main",
            });
            return db;
          },
          catch: (error) =>
            new LocalDatabaseError({
              message: "Failed to create internal database",
              cause: error,
            }),
        }),
        (db) => Effect.promise(() => db.close())
      );

      const localDatabase = yield* Effect.acquireRelease(
        Effect.tryPromise({
          try: async () => {
            const db = new Surreal({
              engines: createWasmEngines({
                capabilities: {
                  experimental: {
                    allow: ["record_references"],
                  },
                },
              }),
              codecOptions: {
                useNativeDates: false,
              },
            });
            await db.connect("mem://");
            await db.use({
              namespace: namespace || "main",
              database: database || localDbName,
            });
            return db;
          },
          catch: (error) =>
            new LocalDatabaseError({
              message: "Failed to create local database",
              cause: error,
            }),
        }),
        (db) => Effect.promise(() => db.close())
      );

      // Create 3 mock nodes instead of connecting to remote
      const node1 = yield* Effect.acquireRelease(
        createMockNode("node1", namespace || "main", database || "test"),
        (node) => Effect.promise(() => node.instance.close())
      );

      const node2 = yield* Effect.acquireRelease(
        createMockNode("node2", namespace || "main", database || "test"),
        (node) => Effect.promise(() => node.instance.close())
      );

      const node3 = yield* Effect.acquireRelease(
        createMockNode("node3", namespace || "main", database || "test"),
        (node) => Effect.promise(() => node.instance.close())
      );

      // Create load balancer
      const loadBalancer = new NodeLoadBalancer([node1, node2, node3]);

      return DatabaseService.of({
        useLocal: useLocalDatabase(localDatabase),
        useInternal: useLocalDatabase(internalDatabase),
        useRemote: useMockNode(loadBalancer),
      });
    })
  );

/**
 * Helper function to create a test database service with 3 nodes
 */
export const createMockDatabaseService = (
  namespace = "test",
  database = "test"
) => {
  return Effect.gen(function* () {
    const node1 = yield* createMockNode("node1", namespace, database);
    const node2 = yield* createMockNode("node2", namespace, database);
    const node3 = yield* createMockNode("node3", namespace, database);

    const loadBalancer = new NodeLoadBalancer([node1, node2, node3]);

    return {
      loadBalancer,
      nodes: [node1, node2, node3],
      cleanup: Effect.all([
        Effect.promise(() => node1.instance.close()),
        Effect.promise(() => node2.instance.close()),
        Effect.promise(() => node3.instance.close()),
      ]),
    };
  });
};
