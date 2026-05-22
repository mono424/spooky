# LIVE feeds misfire when permission rules dereference related records

**Observed on:** SurrealDB `v3.0.5`. Upstream issues `#3602` and `#4026`.

## Symptom

A LIVE query on a table whose `PERMISSIONS FOR select` rule walks through a related record's field — e.g. `out.published = true OR out.author.id = $auth.id` — fails to deliver notifications correctly. The notification path either:

- never fires, or
- fires with stale data (the related record's old field values), or
- fires from the wrong session.

The exact failure mode depends on whether the dereferenced row was updated, deleted, or replaced.

## Reproduction

```surql
DEFINE TABLE comment SCHEMALESS
  PERMISSIONS FOR select
    WHERE thread.published = true
       OR ($access = 'account' AND thread.author.id = $auth.id);
DEFINE FIELD thread ON comment TYPE record<thread>;

-- Authenticated record-user session opens:
LIVE SELECT * FROM comment;

-- Another session creates a thread (published=false), then a comment under it,
-- then flips the thread to published=true.
-- The LIVE on the first session is expected to start delivering comments
-- once the parent thread is published. In practice it doesn't reliably.
```

The dereference-through-`thread.published` is the trigger.

## Workaround

CRDT cross-browser delivery doesn't subscribe to its own LIVE on `_00_crdt` / `_00_cursor`. Instead it rides the parent table's LIVE feed: on every CRDT meta-row UPSERT the writer also bumps the parent's `_00_rv` (a no-op assignment that triggers the parent's existing LIVE). Receivers detect the parent-row LIVE event and pull matching `_00_crdt` / `_00_cursor` rows via subquery against the parent's id. Permission inheritance is enforced server-side via `record_id.id != NONE` and `fn::can_update_record`, both of which avoid the dereference-permission shape.

**Files changed:**
- `packages/core/src/modules/crdt/index.ts` — module-level comment documents the indirection; LIVE subscription is on the parent table, not the CRDT/cursor tables themselves. See `CrdtManager.liveByTable`.
- Schema-side: permission rules on `_00_crdt` / `_00_cursor` use `record_id.id != NONE` (presence check) rather than dereferenced predicates.
