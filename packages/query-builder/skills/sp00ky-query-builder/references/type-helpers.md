# Type Helpers Reference

## Schema Types

### `SchemaStructure`

The top-level schema interface:

```typescript
interface SchemaStructure {
  readonly tables: readonly {
    readonly name: string;
    readonly columns: Record<string, ColumnSchema>;
    readonly primaryKey: readonly string[];
  }[];
  readonly relationships: readonly {
    readonly from: string;
    readonly field: string;
    readonly to: string;
    readonly cardinality: 'one' | 'many';
  }[];
  readonly backends: Record<string, HTTPOutboxBackendDefinition>;
  readonly access?: Record<string, AccessDefinition>;
  readonly buckets?: readonly BucketDefinitionSchema[];
}
```

### `ColumnSchema`

```typescript
interface ColumnSchema {
  readonly type: 'string' | 'number' | 'boolean' | 'null' | 'json';
  readonly optional: boolean;
  readonly dateTime?: boolean;
  readonly recordId?: boolean;
}
```

## Table Type Helpers

| Type | Description |
|------|-------------|
| `TableNames<S>` | Union of all table name strings from the schema |
| `GetTable<S, Name>` | Extract a specific table definition by name |
| `TableModel<T>` | Convert a table's `columns` record into a TypeScript object type |
| `TableFieldNames<T>` | Union of all column names for a table |

### Example

```typescript
type MySchema = typeof schema;
type Tables = TableNames<MySchema>; // 'user' | 'post'
type UserTable = GetTable<MySchema, 'user'>; // The user table definition
type UserModel = TableModel<UserTable>; // { id: string; email: string; name: string | null }
```

## Relationship Type Helpers

| Type | Description |
|------|-------------|
| `TableRelationships<S, TableName>` | All relationship definitions originating from a table |
| `RelationshipFields<S, TableName>` | Union of relationship field names for a table |
| `GetRelationship<S, TableName, Field>` | Get a specific relationship by table and field name |

## Result Type Helpers

| Type | Description |
|------|-------------|
| `QueryResult<S, TableName, RelatedFields, IsOne>` | The full query result type. If `IsOne` is `true`, returns a single object; otherwise an array. Includes related fields merged into the base model. |
| `RelatedFieldsMap` | A record mapping field names to `{ to, cardinality, relatedFields }` |

## Backend Type Helpers

| Type | Description |
|------|-------------|
| `BackendNames<S>` | Union of all backend names |
| `BackendRoutes<S, B>` | Union of all route paths for a backend |
| `RoutePayload<S, B, R>` | The typed payload object for a specific route |
| `BucketNames<S>` | Union of all bucket names |
| `BucketConfig<S, B>` | Configuration for a specific bucket |
