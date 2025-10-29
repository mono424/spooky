import { describe, it, expect, expectTypeOf } from "vitest";
import { QueryBuilder, buildQueryFromOptions } from "./query-builder";
import { RecordId } from "surrealdb";
import type { GenericSchema, RelationshipsMetadata } from "./types";

// Test schema - fields store IDs as strings, relationships hydrate to full objects
interface TestSchema extends GenericSchema {
  user: { id: string; username: string; email: string; created_at: number };
  thread: {
    id: string;
    title: string;
    content: string;
    author: string; // RecordId stored as string
    comments: string[] | null; // Array of RecordIds
    created_at: number;
  };
  comment: {
    id: string;
    content: string;
    author: string; // RecordId stored as string
    thread: string; // RecordId stored as string
    created_at: number;
  };
}

interface TestRelationships extends RelationshipsMetadata {
  thread: {
    author: {
      model: TestSchema["user"];
      table: "user";
      cardinality: "one";
    };
    comments: {
      model: TestSchema["comment"];
      table: "comment";
      cardinality: "many";
    };
  };
  comment: {
    author: {
      model: TestSchema["user"];
      table: "user";
      cardinality: "one";
    };
    thread: {
      model: TestSchema["thread"];
      table: "thread";
      cardinality: "one";
    };
  };
}

const testRelationships: TestRelationships = {
  thread: {
    author: {
      model: {} as TestSchema["user"],
      table: "user",
      cardinality: "one",
    },
    comments: {
      model: {} as TestSchema["comment"],
      table: "comment",
      cardinality: "many",
    },
  },
  comment: {
    author: {
      model: {} as TestSchema["user"],
      table: "user",
      cardinality: "one",
    },
    thread: {
      model: {} as TestSchema["thread"],
      table: "thread",
      cardinality: "one",
    },
  },
};

describe("QueryBuilder", () => {
  it("should build basic SELECT query", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );
    const result = builder.buildQuery();

    expect(result.query).toBe("SELECT * FROM user;");
    expect(result.vars).toEqual({});
  });

  it("should build query with where conditions", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );
    builder.where({ username: "john", email: "john@example.com" });
    const result = builder.buildQuery();

    expect(result.query).toBe(
      "SELECT * FROM user WHERE username = $username AND email = $email;"
    );
    expect(result.vars).toEqual({
      username: "john",
      email: "john@example.com",
    });
  });

  it("should build query with select fields", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );
    builder.select("username", "email");
    const result = builder.buildQuery();

    expect(result.query).toBe("SELECT username, email FROM user;");
  });

  it("should throw error when calling select twice", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );
    builder.select("username");
    expect(() => builder.select("email")).toThrow(
      "Select can only be called once per query"
    );
  });

  it("should build query with ordering, limit, and offset", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );
    builder.orderBy("created_at", "desc").limit(10).offset(5);
    const result = builder.buildQuery();

    expect(result.query).toBe(
      "SELECT * FROM user ORDER BY created_at desc LIMIT 10 START 5;"
    );
  });

  it("should support method chaining", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );
    const result = builder
      .where({ username: "john" })
      .select("username", "email")
      .orderBy("created_at", "desc")
      .limit(10)
      .buildQuery();

    expect(result.query).toBe(
      "SELECT username, email FROM user WHERE username = $username ORDER BY created_at desc LIMIT 10;"
    );
    expect(result.vars).toEqual({ username: "john" });
  });

  it("should build LIVE SELECT query (ignores ORDER BY, LIMIT, START)", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );
    builder
      .where({ username: "john" })
      .orderBy("created_at", "desc")
      .limit(10)
      .offset(5);
    const result = builder.buildLiveQuery();

    expect(result.query).toBe(
      "LIVE SELECT * FROM user WHERE username = $username;"
    );
  });
});

describe("Relationship Queries", () => {
  it("should build query with one-to-one relationship", () => {
    const builder = new QueryBuilder<
      TestSchema,
      TestSchema["thread"],
      "thread",
      typeof testRelationships
    >("thread", testRelationships);
    builder.related("author");
    const result = builder.buildQuery();

    expect(result.query).toBe(
      "SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author FROM thread;"
    );
  });

  it("should build query with one-to-many relationship", () => {
    const builder = new QueryBuilder<
      TestSchema,
      TestSchema["thread"],
      "thread",
      typeof testRelationships
    >("thread", testRelationships);
    builder.related("comments");
    const result = builder.buildQuery();

    expect(result.query).toBe(
      "SELECT *, (SELECT * FROM comment WHERE thread=$parent.id) AS comments FROM thread;"
    );
  });

  it("should build query with relationship modifiers", () => {
    const builder = new QueryBuilder<
      TestSchema,
      TestSchema["thread"],
      "thread",
      typeof testRelationships
    >("thread", testRelationships);
    builder.related("comments", (q) =>
      q.where({ author: "user:123" }).limit(5)
    );
    const result = builder.buildQuery();

    expect(result.query).toBe(
      "SELECT *, (SELECT * FROM comment WHERE thread=$parent.id AND author = user:⟨123⟩ LIMIT 5) AS comments FROM thread;"
    );
  });

  it("should build query with nested relationships", () => {
    const builder = new QueryBuilder<
      TestSchema,
      TestSchema["thread"],
      "thread",
      typeof testRelationships
    >("thread", testRelationships);
    builder.related("comments", (q) => q.related("author"));
    const result = builder.buildQuery();

    expect(result.query).toBe(
      "SELECT *, (SELECT *, (SELECT * FROM user WHERE id=$parent.author LIMIT 1)[0] AS author FROM comment WHERE thread=$parent.id) AS comments FROM thread;"
    );
  });
});

describe("buildQueryFromOptions", () => {
  it("should build query from options", () => {
    const result = buildQueryFromOptions<TestSchema["user"]>("SELECT", "user", {
      where: { username: "john" },
      select: ["username", "email"],
      orderBy: { username: "desc" },
      limit: 10,
    });

    expect(result.query).toBe(
      "SELECT username, email FROM user WHERE username = $username ORDER BY username desc LIMIT 10;"
    );
    expect(result.vars).toEqual({ username: "john" });
  });

  it("should build LIVE SELECT query from options", () => {
    const result = buildQueryFromOptions<TestSchema["user"]>(
      "LIVE SELECT",
      "user",
      {
        where: { username: "john" },
      }
    );

    expect(result.query).toBe(
      "LIVE SELECT * FROM user WHERE username = $username;"
    );
  });
});

describe("RecordId Parsing", () => {
  it("should parse string IDs to RecordId", () => {
    const builder = new QueryBuilder<
      TestSchema,
      TestSchema["thread"],
      "thread"
    >("thread");
    builder.where({ author: "user:123", id: "abc123" });
    const result = builder.buildQuery();

    expect(result.vars).toBeDefined();
    expect(result.vars!.author).toBeInstanceOf(RecordId);
    expect((result.vars!.author as RecordId).toString()).toBe("user:⟨123⟩");
    expect(result.vars!.id).toBeInstanceOf(RecordId);
    expect((result.vars!.id as RecordId).toString()).toBe("thread:abc123");
  });

  it("should not parse non-ID strings", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );
    builder.where({ username: "john_doe" });
    const result = builder.buildQuery();

    expect(result.vars?.username).toBe("john_doe");
    expect(result.vars?.username).not.toBeInstanceOf(RecordId);
  });
});

describe("Edge Cases", () => {
  it("should handle empty where object", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );
    builder.where({});
    const result = builder.buildQuery();

    expect(result.query).toBe("SELECT * FROM user;");
  });

  it("should handle special characters in strings", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );
    builder.where({ username: 'john"doe' });
    const result = builder.buildQuery();

    expect(result.vars?.username).toBe('john"doe');
  });

  it("should return options via getOptions()", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );
    builder.where({ username: "john" }).limit(10);
    const options = builder.getOptions();

    expect(options.where).toEqual({ username: "john" });
    expect(options.limit).toBe(10);
  });
});

describe("Type Tests", () => {
  it("should enforce correct table names", () => {
    // Valid table names should work
    new QueryBuilder<TestSchema, TestSchema["user"], "user">("user");
    new QueryBuilder<TestSchema, TestSchema["thread"], "thread">("thread");
    new QueryBuilder<TestSchema, TestSchema["comment"], "comment">("comment");

    // @ts-expect-error - invalid table name should not compile
    new QueryBuilder<TestSchema, TestSchema["user"], "user">("invalid_table");
  });

  it("should enforce correct field names in where clause", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );

    // Valid fields should work
    builder.where({ username: "john" });
    builder.where({ email: "john@example.com" });
    builder.where({ id: "user:123" });

    // @ts-expect-error - invalid field should not compile
    builder.where({ invalid_field: "value" });
  });

  it("should enforce correct field names in select", () => {
    const builder1 = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );
    const builder2 = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );

    // Valid fields should work
    builder1.select("username", "email");
    builder2.select("id", "created_at");

    const builder3 = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );
    // @ts-expect-error - invalid field should not compile
    builder3.select("invalid_field");
  });

  it("should enforce correct field names in orderBy", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );

    // Valid fields should work
    builder.orderBy("username", "asc");
    builder.orderBy("created_at", "desc");

    // @ts-expect-error - invalid field should not compile
    builder.orderBy("invalid_field", "asc");
  });

  it("should enforce correct relationship field names", () => {
    const builder = new QueryBuilder<
      TestSchema,
      TestSchema["thread"],
      "thread",
      typeof testRelationships
    >("thread", testRelationships);

    // Valid relationship fields should work
    builder.related("author");
    builder.related("comments");
  });

  it("should enforce relationship metadata types", () => {
    const builder = new QueryBuilder<
      TestSchema,
      TestSchema["thread"],
      "thread",
      typeof testRelationships
    >("thread", testRelationships);

    // The related method should accept a modifier function with correct types
    builder.related("comments", (q) => {
      // Should be able to call methods on the related query builder
      q.where({ content: "test" });
      q.limit(5);
      return q;
    });

    builder.related("author", (q) => {
      // Should be able to call methods on the related query builder
      q.where({ username: "john" });
      return q;
    });
  });

  it("should enforce correct types in where clause values", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );

    // String fields should accept strings
    builder.where({ username: "john" });
    builder.where({ email: "test@example.com" });

    // Number fields should accept numbers
    builder.where({ created_at: 123456 });

    // ID fields should accept strings (will be parsed to RecordId)
    builder.where({ id: "user:123" });
  });

  it("should return correctly typed query result", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );
    const result = builder.buildQuery();

    // Query should be a string
    expectTypeOf(result.query).toBeString();

    // Vars should be a record or undefined
    expectTypeOf(result.vars).toEqualTypeOf<
      Record<string, unknown> | undefined
    >();
  });

  it("should enforce correct select return type", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );

    // Select should return the builder for chaining
    const result = builder.select("username", "email");
    expectTypeOf(result).toMatchTypeOf<typeof builder>();
  });

  it("should enforce correct method chaining types", () => {
    const builder = new QueryBuilder<TestSchema, TestSchema["user"], "user">(
      "user"
    );

    // All methods should return the builder for chaining
    const result = builder
      .where({ username: "john" })
      .select("username", "email")
      .orderBy("created_at", "desc")
      .limit(10)
      .offset(5);

    expectTypeOf(result).toMatchTypeOf<typeof builder>();
  });

  it("should enforce buildQueryFromOptions parameter types", () => {
    // Valid options should work
    buildQueryFromOptions<TestSchema["user"]>("SELECT", "user", {
      where: { username: "john" },
      limit: 10,
    });

    buildQueryFromOptions<TestSchema["thread"]>("LIVE SELECT", "thread", {
      where: { title: "test" },
    });
  });
});
