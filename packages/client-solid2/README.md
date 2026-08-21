# @spooky-sync/client-solid2

Solid **2.0** native bindings for the Sp00ky reactive local-first SurrealDB framework.

Counterpart of `@spooky-sync/client-solid` (Solid 1.x); the two coexist until Solid 2.0 is stable. Peer deps: `solid-js@^2.0.0-rc.0` + `@solidjs/signals@^2.0.0-rc.0` (coordinated RC — pin matching versions, apps also need `@solidjs/web`).

What "native" means here:

- Query results are a `createProjection` fed by an async generator over the engine's live subscription: keyed reconcile by `id`, row identity preserved, coarse `<For>` readers notified — no manual reconcile/version-signal plumbing.
- `createQuery` exposes both worlds: non-suspending accessors (`data`, `isLoading`, `isFetching`, `isSettled`, `error`) and a suspending `ready()` for `<Loading>` boundaries. Born committed (`seedLoadingValue`), so local-first cache paints never suspend.
- Status hooks (`useSyncStatus`, `useStorageStatus`, `usePendingMutations`, feature flags, app release) are async-iterable-backed memos with `loadingValue`.
- Mutations stay plain async calls — the engine is already optimistic local-first end to end (local commit → live re-emit → outbox sync). `createSubmission` adds button pending/error state.

See `QUICK_START.md` for usage and the migration table from client-solid, and `src/lib/__tests__/rc-semantics.test.ts` for the probed Solid 2 rc contracts this package depends on (run them after every Solid version bump).

```bash
pnpm --filter @spooky-sync/client-solid2 test
pnpm --filter @spooky-sync/client-solid2 build
```
