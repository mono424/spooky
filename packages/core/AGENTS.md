# `@spooky-sync/core` — agent guide

## What this package is

The sync engine. A `Sp00kyClient<S>` owns a local database (memory or IndexedDB), a remote SurrealDB connection, a mutation queue, and a CRDT manager. It's framework-agnostic — UI bindings live in `@spooky-sync/client-solid` (or future `client-react`, etc.). Most app code touches this package indirectly through a UI binding's `useDb()` hook.

## Mental model

```
.surql schema
   │  spky generate
   ▼
schema.gen.ts (typed schema + SURQL_SCHEMA constant)
   │  passed to SyncedDbConfig
   ▼
Sp00kyClient<S>  ── local store (memory | IndexedDB)
       │            ↑↓ DBSP reactive query layer
       │            ↑↓ mutation queue
       └── SSP ── remote SurrealDB
```

Local mutations are applied optimistically, ingested into a DBSP layer that drives reactive query updates, then drained to the remote via SSP. Live updates flow back the same path.

## Key exports (`src/index.ts`)

- `Sp00kyClient<S>` — main class. Methods: `init()`, `create(id, payload)`, `update(table, id, payload, options?)`, `delete(table, idOrSelector)`, `query(table, opts?)`, `run(backend, route, payload)`, `bucket(name)`, `useRemote(fn)`, `authenticate(token)`, `signOut()`. Plus `pendingMutationCount` and `subscribeToPendingMutations(cb)`.
- `BucketHandle` — file storage handle (`put`, `get`, `delete`, `exists`).
- `AuthService` — token management, sign-in/sign-out events.
- `CrdtManager`, `CrdtField`, `cursorColorFromName`, `CURSOR_COLORS` — Loro-CRDT integration.
- Types: `Sp00kyConfig`, `SyncedDbConfig` (re-exported by client-solid as the consumer-facing shape), `QueryTimeToLive`, `PersistenceClient`, `StoreType`, `UpdateOptions`, `RunOptions`.
- Subpath: `@spooky-sync/core/otel` — `createOtelTransmit(endpoint)` for piping pino logs to OpenTelemetry.

## Common gotchas

- **Use UI hooks, not the client directly.** In a Solid app, call `useDb()` from `@spooky-sync/client-solid`. Touch `Sp00kyClient` only inside `provider.client` for advanced flows.
- **Mutations are optimistic.** `db.create` / `db.update` / `db.delete` return immediately; the queue drains in the background. Inspect progress via `pendingMutationCount`.
- **Record IDs are full strings.** `db.create('thread:abc123', {...})` — *not* `db.create('thread', { id: 'abc123' })`. The first arg is `'<table>:<id>'`. Generate IDs with `Uuid` from `surrealdb` (re-exported by `client-solid`).
- **`db.update` takes `(table, id, payload, options?)`**. The `debounced` option (`{ debounced: true }` or `{ debounced: { delay, key } }`) coalesces rapid updates — use it for CRDT text/title fields.
- **`db.useRemote(fn)` bypasses the cache.** Use only for queries that can't be expressed via the query builder (e.g. graph traversals across `RELATE` edges). Results are not synced into the local cache.
- **`@parent` columns are auto-populated.** Defined in the `.surql` schema as `record<user>` with the `-- @parent` annotation; never write them yourself — they're set from the auth context server-side.

## Module map (under `src/modules/`)

- `data/` — local store + DBSP reactive query layer.
- `sync/` — SSP client, mutation queue, live-update ingestion.
- `cache/` — query result cache with TTL.
- `crdt/` — Loro CRDT manager for collaborative text fields.
- `auth/` — token storage, sign-in flow.
- `devtools/` — devtools bridge (talks to the browser extension and `@spooky-sync/devtools-mcp`).

## Pointers

- UI bindings: `node_modules/@spooky-sync/client-solid/AGENTS.md`
- Query builder DSL: `node_modules/@spooky-sync/query-builder/AGENTS.md`
- Schema codegen / migrations: `node_modules/@spooky-sync/cli/AGENTS.md`
- Live introspection: `node_modules/@spooky-sync/devtools-mcp/AGENTS.md`
