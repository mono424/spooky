# Live Query Architecture Documentation

## Overview

This document describes the live query architecture implemented in `db-solid`, a SurrealDB client with automatic cache synchronization and live query support.

## Table of Contents

- [Architecture Overview](#architecture-overview)
- [Core Components](#core-components)
- [Data Flow](#data-flow)
- [Query Lifecycle](#query-lifecycle)
- [Deduplication System](#deduplication-system)
- [API Reference](#api-reference)
- [Usage Examples](#usage-examples)
- [Performance Considerations](#performance-considerations)

## Architecture Overview

The live query system is built on three key principles:

1. **Remote Live Queries**: All live queries are executed on the remote SurrealDB server
2. **Local Cache**: Local SurrealDB WASM instance acts as a reactive cache
3. **Automatic Synchronization**: Changes from remote are automatically synced to local cache

```
┌─────────────────────────────────────────────────────────────┐
│                      Application Layer                       │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐      │
│  │  Component A │  │  Component B │  │  Component C │      │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘      │
│         │                  │                  │              │
│         └──────────────────┼──────────────────┘              │
│                            │                                 │
└────────────────────────────┼─────────────────────────────────┘
                             │
                             │ .query()
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                    Query Layer (TableQuery)                  │
│  ┌──────────────────────────────────────────────────────┐   │
│  │            LiveQueryList (Per Query)                 │   │
│  │  - Hydration from local cache                        │   │
│  │  - Subscribe to syncer for updates                   │   │
│  └──────────────────────────────────────────────────────┘   │
└────────────────────────────┬─────────────────────────────────┘
                             │
                             │ subscribeLiveQuery()
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                    Syncer (Central Hub)                      │
│  ┌──────────────────────────────────────────────────────┐   │
│  │  Query Tracking & Deduplication                      │   │
│  │  - Map<queryKey, TrackedLiveQuery>                   │   │
│  │  - Reference counting                                │   │
│  │  - Listener management                               │   │
│  └──────────────────────────────────────────────────────┘   │
│                             │                                │
│         ┌───────────────────┼───────────────────┐            │
│         │                   │                   │            │
│         ▼                   ▼                   ▼            │
│  ┌──────────┐        ┌──────────┐        ┌──────────┐       │
│  │  Query 1 │        │  Query 2 │        │  Query 3 │       │
│  │ refCount │        │ refCount │        │ refCount │       │
│  │    = 2   │        │    = 1   │        │    = 3   │       │
│  └──────────┘        └──────────┘        └──────────┘       │
└────────────────────────────┬─────────────────────────────────┘
                             │
                             │ LIVE SELECT (remote)
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                     Remote SurrealDB Server                  │
│  ┌──────────────────────────────────────────────────────┐   │
│  │              Live Query Subscriptions                │   │
│  │  - Watches for data changes                          │   │
│  │  - Sends CREATE/UPDATE/DELETE events                 │   │
│  └──────────────────────────────────────────────────────┘   │
└────────────────────────────┬─────────────────────────────────┘
                             │
                             │ Live Events
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                  Syncer (Event Handler)                      │
│  1. Receives event from remote                               │
│  2. Updates local cache (INSERT/UPDATE/DELETE)               │
│  3. Notifies all listeners for that query                    │
└────────────────────────────┬─────────────────────────────────┘
                             │
                             │ notify listeners
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                  LiveQueryList (Re-hydration)                │
│  - Re-fetch from local cache                                 │
│  - Update reactive proxy state                               │
└────────────────────────────┬─────────────────────────────────┘
                             │
                             │ state update
                             ▼
┌─────────────────────────────────────────────────────────────┐
│                      UI Update (Solid.js)                    │
│  - createEffect detects proxy change                         │
│  - Component re-renders with new data                        │
└─────────────────────────────────────────────────────────────┘
```

## Core Components

### 1. Syncer (`syncer.ts`)

The central hub that manages all remote live queries and cache synchronization.

**Key Responsibilities:**
- Track active live queries with reference counting
- Deduplicate identical queries
- Subscribe to remote live query events
- Update local cache when remote data changes
- Notify all listeners when data updates

**Data Structures:**

```typescript
interface TrackedLiveQuery {
  queryKey: string;              // Unique identifier for the query
  subscription: LiveSubscription; // SurrealDB live subscription
  query: string;                  // Original SELECT query
  vars?: Record<string, unknown>; // Query variables
  refCount: number;               // Number of active subscribers
  affectedTables: Set<string>;    // Tables this query reads from
}

class Syncer {
  // Maps queryKey → TrackedLiveQuery
  private liveQueries: Map<string, TrackedLiveQuery>;

  // Maps tableName → Set<queryKey> for efficient lookups
  private tableToQueryKeys: Map<string, Set<string>>;

  // Maps queryKey → Set<listener callbacks>
  private queryListeners: Map<string, Set<() => void>>;
}
```

**Key Methods:**

```typescript
// Subscribe to a live query (with deduplication)
async subscribeLiveQuery(
  query: string,
  vars: Record<string, unknown> | undefined,
  affectedTables: string[],
  onUpdate: () => void
): Promise<() => void>

// Unsubscribe (decrements refCount, kills if zero)
private unsubscribeLiveQuery(
  queryKey: string,
  listener: () => void
): void

// Handle remote live query events
private async handleRemoteUpdate(
  queryKey: string,
  event: LiveMessage
): Promise<void>
```

### 2. LiveQueryList (`table-queries.ts`)

Manages individual query instances, handling hydration and reactive updates.

**Key Responsibilities:**
- Initial hydration from local cache
- Subscribe to syncer for remote updates
- Re-hydrate when notified of changes
- Manage reactive proxy state

**Lifecycle:**

```
┌──────────────┐
│   init()     │ ────┐
└──────────────┘     │
                     │
         ┌───────────▼──────────┐
         │   1. hydrate()       │
         │   (from local cache) │
         └───────────┬──────────┘
                     │
         ┌───────────▼──────────────┐
         │  2. initRemoteLive()     │
         │  (subscribe via syncer)  │
         └───────────┬──────────────┘
                     │
         ┌───────────▼──────────────┐
         │  3. Wait for events...   │
         └───────────┬──────────────┘
                     │
         ┌───────────▼──────────────┐
         │  4. onUpdate callback    │
         │  → Re-hydrate from cache │
         └───────────┬──────────────┘
                     │
                     │ (repeat)
                     │
         ┌───────────▼──────────────┐
         │   kill()                 │
         │   (unsubscribe)          │
         └──────────────────────────┘
```

**Code Structure:**

```typescript
export class LiveQueryList<Schema, Model> {
  private state: Model[];                    // Reactive proxy array
  private unsubscribe: (() => void) | undefined; // Cleanup function

  constructor(
    private hydrationQuery: QueryInfo,       // SELECT query for cache
    private tableName: string,               // Primary table name
    private db: SyncedDb<Schema>,           // Database instance
    private callback: (items: Model[]) => void // Update callback
  ) {}

  private async hydrate(): Promise<void> {
    // Fetch from local cache
    const [models] = await this.db
      .queryLocal(this.hydrationQuery.query, this.hydrationQuery.vars)
      .collect();
    this.state = models as Model[];
    this.callback(this.state);
  }

  private async initRemoteLive(): Promise<void> {
    const syncer = this.db.getSyncer();
    if (!syncer) return;

    // Subscribe to remote live query
    this.unsubscribe = await syncer.subscribeLiveQuery(
      this.hydrationQuery.query,
      this.hydrationQuery.vars,
      [this.tableName],
      async () => await this.hydrate() // Re-hydrate on change
    );
  }

  public async init(): Promise<void> {
    await this.hydrate();
    await this.initRemoteLive();
  }

  public kill(): void {
    this.unsubscribe?.();
  }
}
```

### 3. ReactiveQueryResult (`table-queries.ts`)

The public API that components interact with, providing a reactive data array.

**Key Features:**
- Exposes a reactive `data` property (Valtio proxy)
- Provides `kill()` method for cleanup
- Automatically updates when underlying LiveQueryList changes

```typescript
export class ReactiveQueryResult<Model> {
  private state: Model[];  // Valtio proxy array
  private liveQuery: LiveQueryList | null = null;

  get data(): Model[] {
    return this.state; // Components access this
  }

  kill(): void {
    this.liveQuery?.kill(); // Cleanup
  }
}
```

## Data Flow

### Query Creation Flow

```
Component calls db.query.thread.find({}).query()
    │
    ▼
QueryBuilder builds query options
    │
    ▼
TableQuery.liveQuery() creates LiveQueryList
    │
    ├─► 1. Build SELECT query for hydration
    │
    ├─► 2. Create LiveQueryList instance
    │       │
    │       ├─► hydrate() - Fetch from local cache
    │       │
    │       └─► initRemoteLive() - Subscribe to syncer
    │           │
    │           └─► syncer.subscribeLiveQuery()
    │               │
    │               ├─► Check if query already exists
    │               │   │
    │               │   ├─► YES: Increment refCount, add listener
    │               │   │
    │               │   └─► NO: Create remote LIVE SELECT
    │               │       │
    │               │       ├─► Execute LIVE SELECT on remote
    │               │       │
    │               │       ├─► Get LiveSubscription
    │               │       │
    │               │       ├─► Subscribe to events
    │               │       │
    │               │       └─► Store in tracking maps
    │               │
    │               └─► Return unsubscribe function
    │
    └─► 3. Wrap in ReactiveQueryResult
        │
        └─► Return to component
```

### Update Flow

```
User creates/updates data remotely
    │
    ▼
Remote SurrealDB processes change
    │
    ▼
Live query detects change
    │
    ▼
Event sent to subscription
    │
    ▼
Syncer.handleRemoteUpdate()
    │
    ├─► Extract record ID and data
    │
    ├─► Update local cache based on action:
    │   ├─► CREATE: localDb.insert()
    │   ├─► UPDATE: localDb.update().merge()
    │   └─► DELETE: localDb.delete()
    │
    └─► Notify all listeners for this query
        │
        ▼
    LiveQueryList.onUpdate callback
        │
        ▼
    Re-hydrate from local cache
        │
        ▼
    Update reactive proxy state
        │
        ▼
    Component's createEffect() detects change
        │
        ▼
    UI re-renders with new data
```

## Query Lifecycle

### Complete Lifecycle Diagram

```
┌───────────────────────────────────────────────────────────┐
│                    Component Mount                         │
└───────────────────┬───────────────────────────────────────┘
                    │
                    ▼
┌───────────────────────────────────────────────────────────┐
│  const liveQuery = await db.query.thread.find().query()   │
└───────────────────┬───────────────────────────────────────┘
                    │
    ┌───────────────┴───────────────┐
    │                               │
    ▼                               ▼
┌─────────┐                  ┌──────────────┐
│ Hydrate │                  │ Check Syncer │
│  from   │                  │   for this   │
│  Local  │                  │    query     │
│  Cache  │                  └──────┬───────┘
└────┬────┘                         │
     │                              │
     │                    ┌─────────┴─────────┐
     │                    │                   │
     │                    ▼                   ▼
     │         ┌──────────────────┐  ┌───────────────┐
     │         │ Query EXISTS     │  │ Query NEW     │
     │         │ refCount++       │  │ Create remote │
     │         │ Add listener     │  │ LIVE SELECT   │
     │         └────────┬─────────┘  └───────┬───────┘
     │                  │                    │
     │                  └──────────┬─────────┘
     │                             │
     ▼                             ▼
┌────────────────────────────────────────┐
│      Initial Render with Data          │
└────────────────┬───────────────────────┘
                 │
                 │ (Component stays mounted)
                 │
    ┌────────────┴────────────┐
    │                         │
    ▼                         ▼
┌────────────┐         ┌─────────────┐
│ Remote     │         │ Component   │
│ Data       │         │ Continues   │
│ Changes    │         │ Using       │
│            │         │ Live Data   │
└─────┬──────┘         └─────────────┘
      │
      ▼
┌────────────────────────────┐
│ Live Event from Remote     │
└────────┬───────────────────┘
         │
         ▼
┌────────────────────────────┐
│ Syncer Updates Local Cache │
└────────┬───────────────────┘
         │
         ▼
┌────────────────────────────┐
│ Notifies All Listeners     │
└────────┬───────────────────┘
         │
         ▼
┌────────────────────────────┐
│ LiveQueryList Re-hydrates  │
└────────┬───────────────────┘
         │
         ▼
┌────────────────────────────┐
│ Proxy State Updates        │
└────────┬───────────────────┘
         │
         ▼
┌────────────────────────────┐
│ Component Re-renders       │
└────────┬───────────────────┘
         │
         │ (cycle repeats)
         │
         ▼
┌────────────────────────────┐
│   Component Unmounts       │
└────────┬───────────────────┘
         │
         ▼
┌────────────────────────────┐
│   liveQuery.kill()         │
└────────┬───────────────────┘
         │
         ▼
┌────────────────────────────┐
│ Syncer Unsubscribe         │
│ - Remove listener          │
│ - Decrement refCount       │
└────────┬───────────────────┘
         │
    ┌────┴────┐
    │         │
    ▼         ▼
┌────────┐  ┌──────────────┐
│refCount│  │ refCount > 0 │
│ == 0   │  │ Keep query   │
└────┬───┘  │ active       │
     │      └──────────────┘
     ▼
┌────────────────────────────┐
│ Kill Remote Live Query     │
│ - subscription.kill()      │
│ - Remove from maps         │
└────────────────────────────┘
```

## Deduplication System

### How Deduplication Works

The syncer uses a query key based on the SQL string and variables to identify identical queries:

```typescript
private getQueryKey(query: string, vars?: Record<string, unknown>): string {
  const varsStr = vars ? JSON.stringify(vars) : "";
  return `${query}|${varsStr}`;
}
```

### Example Scenario

```
Component A: db.query.thread.find({ status: 'active' }).query()
Component B: db.query.thread.find({ status: 'active' }).query()
Component C: db.query.thread.find({ status: 'archived' }).query()
```

**Resulting State:**

```
liveQueries Map:
┌──────────────────────────────────────────────────────────┐
│ Query Key                                  │ Ref Count   │
├────────────────────────────────────────────┼─────────────┤
│ "SELECT * FROM thread WHERE status = ...│1" │ 2 (A & B)   │
│ "SELECT * FROM thread WHERE status = ...│2" │ 1 (C)       │
└──────────────────────────────────────────────────────────┘

queryListeners Map:
┌──────────────────────────────────────────────────────────┐
│ Query Key 1                │ Listeners                   │
├────────────────────────────┼─────────────────────────────┤
│ SELECT...status='active'   │ Set { listenerA, listenerB }│
│ SELECT...status='archived' │ Set { listenerC }           │
└──────────────────────────────────────────────────────────┘

Remote Server:
┌──────────────────────────────────────────────────────────┐
│ Only 2 live queries are active (not 3!)                  │
│ 1. LIVE SELECT * FROM thread WHERE status = 'active'     │
│ 2. LIVE SELECT * FROM thread WHERE status = 'archived'   │
└──────────────────────────────────────────────────────────┘
```

### Benefits of Deduplication

1. **Reduced Network Traffic**: Fewer WebSocket subscriptions
2. **Lower Server Load**: Server tracks fewer live queries
3. **Better Performance**: Less memory and CPU usage
4. **Consistency**: All components see the same data simultaneously

## API Reference

### SyncedDb

```typescript
class SyncedDb<Schema> {
  // Get the syncer instance
  getSyncer(): Syncer | null;

  // Query namespace for type-safe table access
  readonly query: TableQueries<Schema>;
}
```

### TableQuery

```typescript
class TableQuery<Schema, Model> {
  // Start a fluent query
  find(where?: Partial<Model>): QueryBuilder<Schema, Model>;

  // Create records on remote server
  createRemote(data: Values<Model> | Values<Model>[]): Promise<Model[]>;

  // Update records on remote server
  updateRemote(recordId: RecordId, data: Partial<Model>): Promise<any>;

  // Delete records on remote server
  deleteRemote(recordId: RecordId): Promise<void>;
}
```

### QueryBuilder

```typescript
class QueryBuilder<Schema, Model> {
  // Add where conditions
  where(conditions: Partial<Model>): this;

  // Select specific fields
  select(...fields: (keyof Model | "*")[]): this;

  // Order results
  orderBy(field: keyof Model, direction: "asc" | "desc"): this;

  // Limit results
  limit(count: number): this;

  // Set offset
  offset(count: number): this;

  // Execute query and return reactive result
  async query(): Promise<ReactiveQueryResult<Model>>;
}
```

### ReactiveQueryResult

```typescript
class ReactiveQueryResult<Model> {
  // Reactive data array (Valtio proxy)
  get data(): Model[];

  // Stop live updates and cleanup
  kill(): void;
}
```

### Syncer

```typescript
class Syncer {
  // Initialize the syncer
  async init(): Promise<void>;

  // Subscribe to a live query (internal, called by LiveQueryList)
  async subscribeLiveQuery(
    query: string,
    vars: Record<string, unknown> | undefined,
    affectedTables: string[],
    onUpdate: () => void
  ): Promise<() => void>;

  // Check if syncer is active
  isActive(): boolean;

  // Destroy all live queries
  async destroy(): Promise<void>;
}
```

## Usage Examples

### Basic Query

```typescript
import { db } from './db';
import { createEffect, onCleanup } from 'solid-js';

function ThreadList() {
  const [threads, setThreads] = createSignal([]);

  onMount(async () => {
    // Create live query
    const liveQuery = await db.query.thread
      .find({})
      .orderBy('created_at', 'desc')
      .query();

    // React to changes
    createEffect(() => {
      setThreads([...liveQuery.data]);
    });

    // Cleanup on unmount
    onCleanup(() => {
      liveQuery.kill();
    });
  });

  return <For each={threads()}>{thread => ...}</For>;
}
```

### Filtered Query

```typescript
const liveQuery = await db.query.thread
  .find({ status: 'active', author: currentUserId })
  .orderBy('created_at', 'desc')
  .limit(20)
  .query();
```

### Creating Records

```typescript
// Create on remote (will trigger live query updates automatically)
await db.query.thread.createRemote({
  title: 'New Thread',
  content: 'Thread content',
  author: userId,
  created_at: new Date()
});

// The live query will automatically:
// 1. Receive event from remote
// 2. Update local cache
// 3. Re-hydrate and update UI
```

### Multiple Components, Same Query

```typescript
// Component A
function ThreadListA() {
  const liveQuery = await db.query.thread.find({ status: 'active' }).query();
  // ...
}

// Component B (different place in app)
function ThreadListB() {
  const liveQuery = await db.query.thread.find({ status: 'active' }).query();
  // ...
}

// Result: Only ONE remote live query is created!
// Both components share the same subscription
// Both update simultaneously when data changes
```

## Performance Considerations

### Memory Usage

```
Per Query:
- TrackedLiveQuery object: ~500 bytes
- Query key string: ~100-200 bytes
- Listener Set: 24 bytes + (8 bytes × number of listeners)
- LiveSubscription (SurrealDB): ~1-2 KB

Example:
10 unique queries × 5 listeners each = ~20-30 KB total
```

### Network Traffic

```
Without Deduplication:
- 100 components with same query = 100 WebSocket subscriptions
- Each event broadcast to 100 connections

With Deduplication:
- 100 components with same query = 1 WebSocket subscription
- Each event broadcast to 1 connection, processed once
- Savings: 99% reduction in network traffic
```

### Best Practices

1. **Always cleanup**: Call `liveQuery.kill()` in `onCleanup()`

```typescript
onMount(async () => {
  const liveQuery = await db.query.thread.find().query();

  onCleanup(() => {
    liveQuery.kill(); // Essential!
  });
});
```

2. **Use specific queries**: More specific queries = better deduplication

```typescript
// Good: Specific query, likely to be reused
db.query.thread.find({ status: 'active' }).orderBy('created_at', 'desc')

// Less optimal: Very specific, unlikely to be reused
db.query.thread.find({ id: specificId })
```

3. **Batch updates on remote**: When creating multiple records, batch them

```typescript
// Good: One remote call
await db.query.thread.createRemote([record1, record2, record3]);

// Less optimal: Multiple remote calls
await db.query.thread.createRemote(record1);
await db.query.thread.createRemote(record2);
await db.query.thread.createRemote(record3);
```

4. **Use pagination**: Limit query results for better performance

```typescript
db.query.thread
  .find()
  .orderBy('created_at', 'desc')
  .limit(50)  // Don't load all records at once
  .query();
```

## Debugging

### Enable Debug Logs

The syncer and live query list log important events to the console:

```
[Syncer] Creating new live query: SELECT * FROM thread|{}
[Syncer] Live query started (refCount: 1): SELECT * FROM thread|{}
[LiveQueryList] Hydrated [{ id: 'thread:1', title: '...' }]
[LiveQueryList] Setting up remote live query { query: '...', table: 'thread' }
[Syncer] Remote update for query: ... { action: 'CREATE', value: { ... } }
[LiveQueryList] Hydrated [{ ... }] (updated)
```

### Common Issues

1. **"No syncer available" warning**
   - Cause: Remote URL not configured
   - Solution: Add `remoteUrl` to `SyncedDbConfig`

2. **Data not updating in UI**
   - Cause: Missing `createEffect()` or not spreading proxy array
   - Solution: Use `createEffect(() => setData([...liveQuery.data]))`

3. **Multiple remote subscriptions for same query**
   - Cause: Different query objects (extra spaces, different var order)
   - Solution: Ensure queries are identical (use same helper function)

4. **Memory leak**
   - Cause: Not calling `liveQuery.kill()` on unmount
   - Solution: Always use `onCleanup(() => liveQuery.kill())`

## Migration Guide

### From Old API to New API

**Before (Local Live Queries):**
```typescript
// ❌ Old way - used local live queries (didn't work well)
const liveQuery = await db.query.thread.find().query();
// Had issues with local WASM SurrealDB live queries
```

**After (Remote Live Queries):**
```typescript
// ✅ New way - uses remote live queries with local cache
const liveQuery = await db.query.thread.find().query();
createEffect(() => {
  setThreads([...liveQuery.data]);
});
// Automatically syncs from remote to local cache
```

**Creating Records:**
```typescript
// ❌ Old way
await db.query.thread.createLocal({ ... });

// ✅ New way - create on remote, auto-syncs to local
await db.query.thread.createRemote({ ... });
```

## Architecture Benefits

### Advantages

1. **Scalability**: Deduplication reduces server load linearly with duplicate queries
2. **Reliability**: Local cache provides instant reads, remote provides consistency
3. **Offline Support**: Local cache works even when remote is temporarily unavailable
4. **Developer Experience**: Simple API, automatic reactivity
5. **Type Safety**: Full TypeScript support with schema types
6. **Performance**: Efficient caching and minimal re-renders

### Trade-offs

1. **Complexity**: More moving parts than direct queries
2. **Memory**: Local cache duplicates some remote data
3. **Latency**: Updates go through cache layer (minimal impact)
4. **Debugging**: More layers to debug compared to simple queries

## Conclusion

This architecture provides a robust, scalable foundation for real-time data synchronization between a remote SurrealDB server and local client cache. The deduplication system ensures efficient resource usage, while the reactive proxy pattern provides a seamless developer experience.

For questions or issues, please refer to the project repository or open an issue.
