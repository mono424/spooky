# SyncedDb Table-Scoped Query API

The `SyncedDb` class now supports table-scoped queries for all schema types automatically.

## Usage

### Basic Setup

```typescript
import { SyncedDb, type TempSchema } from "db-solid";

// Create database instance with schema types
const db = new SyncedDb<TempSchema>({
  localDbName: "my-app-local",
  internalDbName: "my-app-internal",
  storageStrategy: "indexeddb",
  namespace: "main",
  database: "my_app",
});

// Initialize
await db.init();
```

### Table-Scoped Queries

You can now query specific tables using `db.query.<table>`:

```typescript
// Query the user table
const users = await db.query.user.queryLocal<{ result: User[] }>(
  "SELECT * FROM user WHERE username = $username",
  { username: "john" }
);

// Query the thread table
const threads = await db.query.thread.queryLocal<{ result: Thread[] }>(
  "SELECT * FROM thread WHERE author = $userId",
  { userId: "user:123" }
);

// Query the comment table
const comments = await db.query.comment.queryLocal<{ result: Comment[] }>(
  "SELECT * FROM comment WHERE thread_id = $threadId",
  { threadId: "thread:abc" }
);
```

### Remote Queries

The same API works for remote queries:

```typescript
// Query remote user table
const remoteUsers = await db.query.user.queryRemote<{ result: User[] }>(
  "SELECT * FROM user"
);
```

### Direct Database Queries (Still Supported)

The original API still works for backward compatibility:

```typescript
// Direct query on the database
const result = await db.queryLocal<{ result: User[] }>("SELECT * FROM user");
```

## TypeScript Support

The implementation uses:

- **Generics**: `SyncedDb<Schema>` accepts your schema type
- **Proxy Pattern**: Automatically creates table query instances for any table name
- **Type Safety**: Full TypeScript autocomplete for table names based on your schema

## How It Works

1. The `SyncedDb` constructor uses a JavaScript Proxy
2. When you access a property like `db.user`, it creates a `TableQuery` instance
3. The `TableQuery` instance has `queryLocal` and `queryRemote` methods
4. These methods delegate to the parent `SyncedDb` instance
5. Table query instances are cached for performance

## Benefits

✅ **Automatic**: Works for any table in your schema without manual configuration
✅ **Type-Safe**: Full TypeScript support with autocomplete
✅ **Flexible**: Direct database queries still work
✅ **Cached**: Table query instances are reused for performance
✅ **Generic**: Works with any schema type you provide
