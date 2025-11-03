import { beforeAll, beforeEach, describe, expect, it } from "@effect/vitest";
import { type SpookyConfig } from "../src/services/index.js";
import { schema as testSchema, SURQL_SCHEMA } from "./test.schema.js";
import { Effect } from "effect";
import { createMockSpooky } from "./mock-spooky.js";

const mockConfig: SpookyConfig<typeof testSchema> = {
  schema: testSchema,
  schemaSurql: SURQL_SCHEMA,
  remoteUrl: "ws://localhost:8000",
  localDbName: "test-local",
  internalDbName: "test-internal",
  storageStrategy: "memory" as const,
  namespace: "test",
  database: "test",
  provisionOptions: {
    force: false,
  },
};

describe("Spooky Initialization", () => {
  it("should initialize spooky with valid config", async () => {
    const { spooky } = await createMockSpooky(mockConfig);

    expect(spooky).toBeDefined();
    expect(spooky.create).toBeDefined();
    expect(spooky.update).toBeDefined();
    expect(spooky.delete).toBeDefined();
    expect(spooky.query).toBeDefined();
  });
});

describe("Mock Database with 3 Nodes", () => {
  beforeAll(async () => {
    const { dbContext } = await createMockSpooky(mockConfig);
    await dbContext.remoteDatabase?.query([SURQL_SCHEMA].join("\n"));

    await dbContext.remoteDatabase?.query(
      [
        "CREATE user CONTENT { id: user:A, username: 'userA', email: 'userA@example.com', password: crypto::argon2::generate('pw1') };",
        "CREATE user CONTENT { id: user:B, username: 'userB', email: 'userB@example.com', password: crypto::argon2::generate('pw2') };",
        "CREATE user CONTENT { id: user:C, username: 'userC', email: 'userC@example.com', password: crypto::argon2::generate('pw3') };",
      ].join("\n")
    );

    await dbContext.remoteDatabase?.query(
      [
        "CREATE thread CONTENT { id: thread:A1, title: 'threadA1', content: 'content', author: user:A, created_at: time::now() };",
        "CREATE thread CONTENT { id: thread:B1, title: 'threadB1', content: 'content', author: user:B, created_at: time::now() };",
        "CREATE thread CONTENT { id: thread:C1, title: 'threadC1', content: 'content', author: user:C, created_at: time::now() };",
      ].join("\n")
    );
  });

  beforeEach(async () => {
    const { spooky } = await createMockSpooky(mockConfig);
    await Effect.runPromise(
      Effect.gen(function* () {
        yield* spooky.clearLocalCache();
      })
    );
  });

  it("should create a query", async () => {
    const { spooky } = await createMockSpooky(mockConfig);

    const query = Effect.runSync(spooky.query("user", {}));
    query.orderBy("created_at", "asc");

    const result = query.build().select();
    expect(result).toBeDefined();
  });

  it("should authenticate", async () => {
    const { spooky, dbContext } = await createMockSpooky(mockConfig);

    const authResponse = await dbContext.remoteDatabase?.signin({
      access: "account",
      variables: {
        username: "userA",
        password: "pw1",
      },
    });

    console.log("authResponse", authResponse);

    expect(authResponse?.token).toBeDefined();

    const userId = await Effect.runPromise(
      spooky.authenticate(authResponse?.token ?? "")
    );

    expect(userId).toBeDefined();
    expect(userId?.id).toBe("A");
  });

  it("should create a query that returns correct remote data", async () => {
    const { spooky, dbContext } = await createMockSpooky(mockConfig);

    const authResponse = await dbContext.remoteDatabase?.signin({
      access: "account",
      variables: {
        username: "userB",
        password: "pw2",
      },
    });

    await Effect.runPromise(spooky.authenticate(authResponse?.token ?? ""));

    const result = Effect.runSync(spooky.query("thread", {})).build().select();
    expect(result).toBeDefined();
    expect(result.data).toHaveLength(0);

    const subResult01 = await new Promise((resolve) =>
      result.subscribe((threads) => resolve(threads))
    );

    const subResult02 = await new Promise((resolve) =>
      result.subscribe((threads) => resolve(threads))
    );

    expect(subResult01).toHaveLength(0);
    expect(subResult02).toHaveLength(3);
  });

  it("should update query when new data is created in remote database", async () => {
    const { spooky, dbContext } = await createMockSpooky(mockConfig);

    const authResponse = await dbContext.remoteDatabase?.signin({
      access: "account",
      variables: {
        username: "userA",
        password: "pw1",
      },
    });

    await Effect.runPromise(spooky.authenticate(authResponse?.token ?? ""));

    const result = Effect.runSync(spooky.query("thread", {}))
      .limit(1)
      .build()
      .select();
    expect(result).toBeDefined();
    expect(result.data).toHaveLength(0);

    const subResult01 = await new Promise((resolve) =>
      result.subscribe((threads) => resolve(threads))
    );

    const subResult02 = await new Promise((resolve) =>
      result.subscribe((threads) => resolve(threads))
    );

    const result2 = Effect.runSync(spooky.query("thread", {}))
      .limit(3)
      .build()
      .select();
    expect(result2).toBeDefined();
    expect(result2.data).toHaveLength(0);

    const subResult03 = await new Promise((resolve) =>
      result2.subscribe((threads) => resolve(threads))
    );

    const subResult04 = await new Promise((resolve) =>
      result2.subscribe((threads) => resolve(threads))
    );

    expect(subResult01).toHaveLength(0);
    expect(subResult02).toHaveLength(1);
    expect(subResult03).toHaveLength(1);
    expect(subResult04).toHaveLength(3);
  });
});
