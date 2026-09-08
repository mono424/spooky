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

- `Sp00kyClient<S>` — main class. Methods: `init()`, `create(id, payload)`, `update(table, id, payload, options?)`, `delete(table, idOrSelector)`, `query(table, opts?)`, `run(backend, route, payload)`, `bucket(name)`, `remoteQuery(sql, vars?)`, `useRemote(fn)`, `authenticate(token)`, `signOut()`. Plus `pendingMutationCount` / `subscribeToPendingMutations(cb)` (outbox depth) and `fetchingQueryCount` / `subscribeToFetchActivity(cb)` (queries mid-fetch, aggregate) for traffic indicators.
- `BucketHandle` — file storage handle (`put`, `get`, `delete`, `exists`), plus the cache-aware `read`, `url`, `pin`/`unpin`, `evict`, `prefetch`. `get` is always remote; `read`/`url` go through the blob cache.
- `services/blobs/` — durable cache for bucket bytes: OPFS holds the files, `_00_blob` holds a manifest that reconcile rebuilds from disk (so a wiped local store costs metadata, not the offline cache). Nothing expires on a timer; eviction is LRU under a byte budget only, skipping pinned and on-screen entries. Config: `blobCache` in `types.ts`.
- `AuthService` — token management, sign-in/sign-out events.
- `CrdtManager`, `CrdtField`, `cursorColorFromName`, `CURSOR_COLORS` — Loro-CRDT integration.
- Types: `Sp00kyConfig`, `SyncedDbConfig` (re-exported by client-solid as the consumer-facing shape), `QueryTimeToLive`, `PersistenceClient`, `StoreType`, `UpdateOptions`, `RunOptions`.
- Subpath: `@spooky-sync/core/otel` — `createOtelTransmit(endpoint)` for piping pino logs to OpenTelemetry.

## Common gotchas

- **Use UI hooks, not the client directly.** In a Solid app, call `useDb()` from `@spooky-sync/client-solid`. Touch `Sp00kyClient` only inside `provider.client` for advanced flows.
- **Mutations are optimistic.** `db.create` / `db.update` / `db.delete` return immediately; the queue drains in the background. Inspect progress via `pendingMutationCount`.
- **Record IDs are full strings.** `db.create('thread:abc123', {...})` — *not* `db.create('thread', { id: 'abc123' })`. The first arg is `'<table>:<id>'`. Generate IDs with `Uuid` from `surrealdb` (re-exported by `client-solid`).
- **`db.update` takes `(table, id, payload, options?)`**. The `debounced` option (`{ debounced: true }` or `{ debounced: { delay, key } }`) coalesces rapid updates — use it for CRDT text/title fields.
- **`db.remoteQuery(sql, vars?)` is the one-shot remote read.** It runs one SurrealQL statement through the client's remote path (connect gate, per-statement timeout, concurrency limit) and returns the SurrealDB result array; results are not synced into the local cache. Use it for counts and other reads the query builder can't express.
- **`db.useRemote(fn)` hands out the bare SDK client.** Same cache bypass, but it skips the connect gate (a call during a reconnect fails with "Specify a namespace to use"). Reserve it for SDK features `remoteQuery` cannot reach (transactions, `live`).
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
