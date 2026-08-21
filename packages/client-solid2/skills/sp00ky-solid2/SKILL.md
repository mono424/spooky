---
name: sp00ky-solid2
description: >-
  Solid 2.0 native integration for the Sp00ky reactive local-first SurrealDB
  framework. Use when an app is on solid-js 2.x (rc or later): setting up
  Sp00kyProvider, createQuery for reactive data (accessor or <Loading>
  suspension style), mutations via plain db calls or createSubmission, status
  hooks, file upload/download. For Solid 1.x apps use the sp00ky-solid skill
  (@spooky-sync/client-solid) instead.
metadata:
  author: sp00ky-sync
  version: "0.0.1"
---

# Sp00ky Solid 2 Client

`@spooky-sync/client-solid2` targets **solid-js 2.0** (`^2.0.0-rc.0`, peer with `@solidjs/signals`; apps also need `@solidjs/web` and `jsxImportSource: "@solidjs/web"`). Coordinated RC: all three Solid packages on matching versions.

Internally the binding is Solid-2 native: query results live in a `createProjection` fed by an async generator over the engine's live subscription (keyed reconcile by `id`, row identity preserved, coarse `<For>` readers notified — no version-signal hack); status hooks are async-iterable-backed memos with `loadingValue`; teardown is `onCleanup`-driven because Solid 2 abandons superseded async generators without terminating them.

## Setup

```tsx
import { Sp00kyProvider } from '@spooky-sync/client-solid2';

<Sp00kyProvider config={{ database: {...}, schema, schemaSurql }} fallback={<Splash />}>
  <MyApp />
</Sp00kyProvider>
```

Props identical to client-solid: `config`, `fallback`, `preload` (awaited gate before revealing UI), `onReady`, `onError`.

## createQuery

```tsx
const posts = createQuery(db.query('post').orderBy('createdAt', 'desc').limit(20).build());
// or reactive: createQuery(() => id() ? db.query('post').where({ id: id() }).one().build() : null)
```

Returns `{ data, ready, error, isLoading, isFetching, isSettled }`:

- `data()` — non-suspending; `[]`/`null` before first result, keyed-reconciled live rows after.
- `ready()` — suspending read for `<Loading fallback={...}>` (Solid 2's renamed Suspense).
- `error()` — registration failure (SSP 503 NOT_READY etc.); never thrown into the tree, sync retries underneath.
- `isSettled()` — fetched AND not fetching; gate windowed-list end detection on this.
- Options: `enabled?: () => boolean`, `deregisterOnCleanup?: boolean`.
- `useQuery` is a deprecated alias of `createQuery`.

Rows are Solid 2 store proxies; class instances (RecordId) are proxied with bound methods. `db.delete('table', row.id)` works; for surrealdb APIs that check `instanceof`, unwrap with `snapshot(row)` first.

## Mutations

Engine is optimistic local-first end to end (local commit → live re-emit → outbox sync; `db.run()` creates an outbox job row `status: 'pending'`). Use plain `db.create/update/delete/run`. For UI pending/error state:

```tsx
const save = createSubmission((t: string) => db.create(`post:${crypto.randomUUID()}`, { title: t }));
save.submit(...); save.pending(); save.error(); save.result(); save.clearError();
```

No `action()`/`createOptimisticStore` layer — deliberate; see create-submission.ts.

## Hooks

Same shapes as client-solid: `useDb`, `usePendingMutations`, `useSyncStatus`, `useStorageStatus`, `useFeatureFlag`, `useAppRelease`, `useCrdtField`, `useFileUpload`, `useDownloadFile`, `createPreload`.

## Testing note

Under node, `solid-js`'s `node` export condition resolves the SSR build where user effects never run. Vitest configs must set `resolve.conditions: ['browser', 'development']` (see this package's vitest.config.ts).
