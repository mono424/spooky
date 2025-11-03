import { describe, expect, it } from "@effect/vitest";
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
    provisionRemote: true,
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
  it("should create a query", async () => {
    const { spooky } = await createMockSpooky(mockConfig);

    const query = Effect.runSync(spooky.query("thread", {}));
    query.orderBy("created_at", "asc");

    const result = query.build().select();
    expect(result).toBeDefined();
  });

  it("should authenticate", async () => {
    const { spooky, dbContext } = await createMockSpooky(mockConfig);

    await dbContext.mockRemoteDatabase?.query(
      [
        "CREATE user CONTENT { id: user:A, username: 'userA', email: 'userA@example.com', password: crypto::argon2::generate('pw1') };",
      ].join("\n")
    );

    const authResponse = await dbContext.mockRemoteDatabase?.signin({
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

  it("should create a query with a filter", async () => {
    const { spooky, dbContext } = await createMockSpooky(mockConfig);

    await dbContext.mockRemoteDatabase?.query(
      [
        "CREATE user CONTENT { id: user:A, username: 'userA', email: 'userA@example.com', password: crypto::argon2::generate('pw1') };",
        "CREATE user CONTENT { id: user:B, username: 'userB', email: 'userB@example.com', password: crypto::argon2::generate('pw2') };",
        "CREATE user CONTENT { id: user:C, username: 'userC', email: 'userC@example.com', password: crypto::argon2::generate('pw3') };",
      ].join("\n")
    );

    const authResponse = await dbContext.mockRemoteDatabase?.signin({
      access: "account",
      variables: {
        username: "userA",
        password: "pw1",
      },
    });

    await Effect.runPromise(spooky.authenticate(authResponse?.token ?? ""));

    await dbContext.mockRemoteDatabase?.query(
      [
        "CREATE thread CONTENT { id: thread:A1, title: 'threadA1', content: 'content', author: thread:A, created_at: time::now() };",
        "CREATE thread CONTENT { id: thread:B1, title: 'threadB1', content: 'content', author: thread:B, created_at: time::now() };",
        "CREATE thread CONTENT { id: thread:C1, title: 'threadC1', content: 'content', author: thread:C, created_at: time::now() };",
      ].join("\n")
    );

    const result = Effect.runSync(spooky.query("thread", {})).build().select();
    await new Promise((resolve) => setTimeout(resolve, 1000));
    console.log(result.data);
    expect(result).toBeDefined();
  });
});
