# LIVE notifications don't fire cross-session on permission-gated tables

**Observed on:** SurrealDB `v3.0.5`, still partially reproducing on `v3.1.0-beta.3` (we keep a poll fallback).

## Symptom

Session A inserts a row into a table whose `PERMISSIONS FOR select` evaluates `true` for session B. Session B holds a LIVE subscription to that table. The expected behavior is that B receives a LIVE notification for the new row. Observed: B's LIVE handler does not fire.

Same-session LIVE notifications work fine. Cross-session LIVE on tables with permissions equal to `true` (no predicate) works fine. The bug only surfaces when there's a non-trivial `PERMISSIONS FOR select` predicate.

Empirically, even with a permission rule that's pinned against a literal record id (so it can't fail), a non-trivial fraction of cross-session deliveries still drop. The pin shrinks the gap but doesn't close it.

## Reproduction

1. As root:

   ```surql
   DEFINE TABLE notes SCHEMALESS
     PERMISSIONS FOR select WHERE $auth.id = user:alice;
   DEFINE ACCESS user_access ON DATABASE TYPE RECORD
     SIGNIN ( SELECT * FROM user WHERE id = $id );
   ```

2. Connect session A as `user:alice`. Open a LIVE: `LIVE SELECT * FROM notes;`.
3. Open session B as root. Insert: `CREATE notes:hello SET text = 'hi';`.

**Observed:** A's LIVE handler doesn't fire even though the row matches A's predicate.

**Expected:** A receives the new row.

A reload-and-SELECT on A returns the row correctly, confirming the data is persisted and visible — only the LIVE delivery is dropped.

## Workaround

Two layers of mitigation in `packages/core/src/modules/sync/sync.ts`:

1. **Dedicated per-user `_00_list_ref_user_<id>` tables.** Each user has their own list_ref table whose permission rule is pinned to a literal record id (`$auth.id = user:<lit>`). The SSP fan-out (`apps/ssp/src/lib.rs::update_all_edges`) writes into the right user's table. Cross-session-same-user LIVE delivery is empirically reliable on this shape.

2. **Periodic poll fallback** at `Sp00kySync.startListRefPoll` (default 500ms, configurable via `Sp00kyConfig.refSyncIntervalMs`). The poll re-fetches the user's `_00_list_ref_user_<id>` and feeds any version bumps through `syncRecords`. Closes the remaining LIVE drops.

**Files changed:**
- `apps/ssp/src/lib.rs::update_all_edges` — routes RELATE/UPDATE/DELETE to the per-user list_ref table.
- `apps/ssp/src/tables.rs` (now `list_ref_tables.rs`) — `ensure_user_tables` DDL on first registration.
- `packages/core/src/modules/sync/sync.ts` — LIVE subscription on `_00_list_ref_user_<id>` plus the 500ms poll loop.
- `packages/core/src/modules/ref-tables.ts` — client-side table-name helper.
