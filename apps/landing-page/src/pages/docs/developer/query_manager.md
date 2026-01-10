---
layout: ../../../layouts/DocsLayout.astro
title: QueryManager
---


The `QueryManager` provides the read-layer interface for the application. It manages **Incantations** (live queries), handles caching, and allows UI components to subscribe to data updates. Importantly, it is strictly **Local-First**: it reads only from the Local Database and never communicates directly with the Remote Database.

## 📦 Responsibility

- **Query Registration**: Registers new queries as "Incantations".
- **Subscription Management**: Allows multiple UI components to listen to the same query hash.
- **Local State**: Maintains the current state of active queries in memory.
- **Event Handling**: Processes updates routed from `SpookySync`.

## 🏗️ Architecture & Boundaries

In the Black Box model, `QueryManager` is the consumer of data:

- **Inputs**:
  - `query()` calls from the UI.
  - `IncantationUpdated` events from `RouterService` (originating from `SpookySync`).
- **Outputs**:
  - `IncantationInitialized` events (to notify Sync/DevTools).
  - Data callbacks to UI subscribers.
- **Local Access**: **YES** (Read-Only perspective, though it writes metadata).
- **Remote Access**: **NO**.

## 🔄 Input/Output Reference

### Public API

| Method                           | Returns              | Description                                                          |
| :------------------------------- | :------------------- | :------------------------------------------------------------------- |
| `query(table, sql, params, ttl)` | `Promise<QueryHash>` | Registers a query. If it exists, returns the existing hash.          |
| `subscribe(hash, callback)`      | `UnsubscribeFn`      | Subscribes a callback function to updates for a specific query hash. |
| `handleIncomingUpdate(event)`    | `void`               | **Internal**: Called by Router to process new data from Sync.        |

### Events (Emitted)

| Event                     | Description                                                 |
| :------------------------ | :---------------------------------------------------------- |
| `IncantationInitialized`  | Emitted when a new query is first created.                  |
| `IncantationUpdated`      | Emitted when data for a query changes (updates UI).         |
| `IncantationTTLHeartbeat` | Emitted periodically to keep the query alive on the server. |

## 🔑 Key Workflows

### 1. Registering a Query

```typescript
const hash = await client.query('SELECT * FROM tasks');
// 1. QueryManager checks if this query hash already exists.
// 2. If NO: Creates an 'Incantation' record in Local DB.
// 3. Emits 'IncantationInitialized'.
// 4. RouterService sees this and tells SpookySync to register it upstream.
// 5. If YES: Just returns the hash and increments the ref count.
```

### 2. Processing Updates

1. `SpookySync` receives data and emits `IncantationUpdated` via `RouterService`.
2. `RouterService` calls `QueryManager.handleIncomingUpdate(payload)`.
3. `QueryManager` looks up the active query by ID.
4. It updates the internal state.
5. It triggers all registered callbacks (UI updates).

## ⚠️ Internal Logic

- **Direct Memory Access**: `QueryManager` holds `activeQueries` in a Map. This allows for O(1) lookup and instant UI updates without hitting IndexedDB for every render cycle.
- **Garbage Collection**: Queries rely on a TTL (Time To Live). `QueryManager` emits heartbeats for active queries. If a query is unused for too long, the server (and eventually the local client) will clean it up.
