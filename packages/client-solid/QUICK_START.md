# Quick Start Guide

## Installation

```bash
pnpm add db-solid
```

## Basic Setup

### 1. Configure Database

```typescript
// db.ts
import { SyncedDb, type SyncedDbConfig } from 'db-solid';
import { type Schema, SURQL_SCHEMA } from './schema.gen';

export const dbConfig: SyncedDbConfig<Schema> = {
  schema: SURQL_SCHEMA,
  localDbName: 'my-app-local',
  internalDbName: 'syncdb-int',
  storageStrategy: 'indexeddb',
  namespace: 'main',
  database: 'my_db',
  remoteUrl: 'http://localhost:8000', // Your SurrealDB server
  tables: ['user', 'thread', 'comment'],
};

export const db = new SyncedDb<Schema>(dbConfig);

export async function initDatabase(): Promise<void> {
  await db.init();
  console.log('Database initialized');
}
```

### 2. Initialize in Your App

```typescript
// App.tsx
import { initDatabase } from "./db";

function App() {
  onMount(async () => {
    await initDatabase();
  });

  return <Router>{/* your routes */}</Router>;
}
```

## Usage Examples

### Querying Data with Live Updates

```typescript
import { db } from "./db";
import { createSignal, createEffect, onMount, onCleanup } from "solid-js";

function ThreadList() {
  const [threads, setThreads] = createSignal([]);

  onMount(async () => {
    // Create a live query that automatically updates
    const liveQuery = await db.query.thread
      .find({})
      .orderBy("created_at", "desc")
      .query();

    // React to changes
    createEffect(() => {
      setThreads([...liveQuery.data]);
    });

    // Cleanup when component unmounts
    onCleanup(() => {
      liveQuery.kill();
    });
  });

  return (
    <For each={threads()}>
      {(thread) => <ThreadCard thread={thread} />}
    </For>
  );
}
```

### Filtered Queries

```typescript
// Find threads by specific criteria
const liveQuery = await db.query.thread
  .find({ status: 'active', author: currentUserId })
  .orderBy('created_at', 'desc')
  .limit(20)
  .query();

// React to changes
createEffect(() => {
  setThreads([...liveQuery.data]);
});
```

### Creating Records

```typescript
// Create a new thread on the remote server
// This will automatically trigger live query updates
async function createThread(title: string, content: string) {
  const [thread] = await db.query.thread.createRemote({
    title,
    content,
    author: currentUserId,
    created_at: new Date(),
  });

  return thread;
}
```

### Updating Records

```typescript
// Update a record on the remote server
async function updateThread(threadId: RecordId, updates: Partial<Thread>) {
  await db.query.thread.updateRemote(threadId, updates);
}
```

### Deleting Records

```typescript
// Delete a record on the remote server
async function deleteThread(threadId: RecordId) {
  await db.query.thread.deleteRemote(threadId);
}
```

## API Cheat Sheet

### Query Builder

```typescript
db.query.tableName
  .find(whereConditions) // Optional: filter records
  .select('field1', 'field2') // Optional: select specific fields
  .orderBy('field', 'asc') // Optional: sort results
  .limit(50) // Optional: limit results
  .offset(10) // Optional: pagination
  .query(); // Execute and return ReactiveQueryResult
```

### CRUD Operations

```typescript
// Create
await db.query.tableName.createRemote(data);
await db.query.tableName.createRemote([data1, data2]); // Batch

// Read (live)
const liveQuery = await db.query.tableName.find().query();

// Update
await db.query.tableName.updateRemote(recordId, updates);

// Delete
await db.query.tableName.deleteRemote(recordId);
```

### Cleanup

```typescript
// Always cleanup live queries when component unmounts
onCleanup(() => {
  liveQuery.kill();
});
```

## How It Works

1. **Query Creation**: When you call `.query()`, a live query is created on the remote server
2. **Initial Data**: Data is fetched from the local cache and displayed immediately
3. **Live Updates**: When data changes on the remote server:
   - The syncer receives the change event
   - Local cache is updated automatically
   - Query re-hydrates from local cache
   - UI updates reactively

```
User Action (createRemote)
    ↓
Remote Server Updates
    ↓
Live Query Event
    ↓
Syncer Updates Local Cache
    ↓
Query Re-hydrates
    ↓
UI Updates (via createEffect)
```

## Key Concepts

### Remote vs Local Operations

- **`createRemote()`**: Creates on remote server, automatically syncs to local cache
- **`createLocal()`**: Creates only in local cache (rarely used)
- **Recommendation**: Always use `Remote` methods for real-time sync

### Query Deduplication

Multiple components with identical queries share the same remote subscription:

```typescript
// Component A
const query1 = await db.query.thread.find({ status: 'active' }).query();

// Component B (different location)
const query2 = await db.query.thread.find({ status: 'active' }).query();

// Result: Only ONE remote live query is created!
// Both components update simultaneously when data changes
```

### Reactive Updates

Use `createEffect` to reactively update when query data changes:

```typescript
const liveQuery = await db.query.thread.find().query();

// ✅ Good: Reactively updates
createEffect(() => {
  setThreads([...liveQuery.data]); // Spread to trigger reactivity
});

// ❌ Bad: Won't update
const threads = liveQuery.data; // Static reference
```

## Common Patterns

### List View

```typescript
function ListView() {
  const [items, setItems] = createSignal([]);

  onMount(async () => {
    const liveQuery = await db.query.item
      .find()
      .orderBy("created_at", "desc")
      .query();

    createEffect(() => setItems([...liveQuery.data]));
    onCleanup(() => liveQuery.kill());
  });

  return <For each={items()}>{(item) => <ItemCard item={item} />}</For>;
}
```

### Detail View

```typescript
function DetailView(props: { id: string }) {
  const [item, setItem] = createSignal(null);

  onMount(async () => {
    const liveQuery = await db.query.item
      .find({ id: new RecordId("item", props.id) })
      .query();

    createEffect(() => setItem(liveQuery.data[0] || null));
    onCleanup(() => liveQuery.kill());
  });

  return <Show when={item()}>{(data) => <ItemDetail item={data()} />}</Show>;
}
```

### Form Submission

```typescript
async function handleSubmit(formData: FormData) {
  try {
    const [newItem] = await db.query.item.createRemote({
      title: formData.title,
      content: formData.content,
      created_at: new Date(),
    });

    // Navigate to detail page or show success
    navigate(`/item/${newItem.id}`);
  } catch (error) {
    console.error('Failed to create item:', error);
  }
}
```

## Troubleshooting

### Data Not Updating

**Problem**: UI doesn't update when data changes

**Solution**: Make sure you're using `createEffect` and spreading the array

```typescript
// ✅ Correct
createEffect(() => {
  setData([...liveQuery.data]); // Spread creates new reference
});

// ❌ Wrong
setData(liveQuery.data); // Same reference won't trigger updates
```

### Memory Leaks

**Problem**: Application gets slower over time

**Solution**: Always cleanup live queries

```typescript
onMount(async () => {
  const liveQuery = await db.query.item.find().query();

  onCleanup(() => {
    liveQuery.kill(); // Essential!
  });
});
```

### "No syncer available" Warning

**Problem**: Warning in console about missing syncer

**Solution**: Ensure `remoteUrl` is configured in your `SyncedDbConfig`

```typescript
export const dbConfig: SyncedDbConfig<Schema> = {
  // ... other config
  remoteUrl: 'http://localhost:8000', // Add this!
};
```

### TypeScript Errors

**Problem**: Type errors when accessing query results

**Solution**: Make sure your schema types are properly generated

```bash
# Generate schema types
pnpm run generate-schema
```

## Best Practices

1. **Always use `createRemote()`** for CRUD operations
2. **Always cleanup** with `liveQuery.kill()` in `onCleanup()`
3. **Use `createEffect()`** to reactively update signals
4. **Spread arrays** when setting state: `[...liveQuery.data]`
5. **Use specific queries** to benefit from deduplication
6. **Paginate large lists** with `.limit()` and `.offset()`
7. **Handle errors** in async operations

## Advanced Usage

### Custom Queries

```typescript
// Use custom SQL for complex queries
const result = await db.queryRemote(
  `SELECT *, author.* FROM thread WHERE created_at > $timestamp`,
  { timestamp: new Date('2024-01-01') }
);

const [threads] = await result.collect();
```

### Authentication

```typescript
// Authenticate with the database
await db.authenticate(jwtToken);
```

### Multiple Conditions

```typescript
const liveQuery = await db.query.thread
  .find({ status: 'active' })
  .where({ category: 'tech' }) // Add more conditions
  .orderBy('votes', 'desc')
  .limit(50)
  .query();
```

## Next Steps

- Read the [Full Architecture Documentation](./LIVE_QUERY_ARCHITECTURE.md)
- Explore the example app in `/example/app-solid`
- Check out the SurrealDB documentation: https://surrealdb.com/docs

## Support

For issues or questions:

- Open an issue on GitHub
- Check the architecture documentation for detailed explanations
- Review the example app for implementation patterns
