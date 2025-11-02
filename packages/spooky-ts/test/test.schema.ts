export const testSchema = {
  tables: [
    {
      name: "user" as const,
      columns: {
        id: { type: "string" as const, optional: false },
        username: { type: "string" as const, optional: false },
        email: { type: "string" as const, optional: false },
        created_at: { type: "number" as const, optional: false },
      },
      primaryKey: ["id"] as const,
    },
    {
      name: "thread" as const,
      columns: {
        id: { type: "string" as const, optional: false },
        title: { type: "string" as const, optional: false },
        content: { type: "string" as const, optional: false },
        author: { type: "string" as const, optional: false },
        comments: { type: "string" as const, optional: true },
        created_at: { type: "number" as const, optional: false },
      },
      primaryKey: ["id"] as const,
    },
    {
      name: "comment" as const,
      columns: {
        id: { type: "string" as const, optional: false },
        content: { type: "string" as const, optional: false },
        author: { type: "string" as const, optional: false },
        thread: { type: "string" as const, optional: false },
        created_at: { type: "number" as const, optional: false },
      },
      primaryKey: ["id"] as const,
    },
  ],
  relationships: [
    {
      from: "thread" as const,
      field: "author" as const,
      to: "user" as const,
      cardinality: "one" as const,
    },
    {
      from: "thread" as const,
      field: "comments" as const,
      to: "comment" as const,
      cardinality: "many" as const,
    },
    {
      from: "comment" as const,
      field: "author" as const,
      to: "user" as const,
      cardinality: "one" as const,
    },
    {
      from: "comment" as const,
      field: "thread" as const,
      to: "thread" as const,
      cardinality: "one" as const,
    },
  ],
} as const;
