# `SELECT *, (subquery) FROM $param` drops the `*` fields

**Observed on:** SurrealDB `v3.0.5`.

## Symptom

When the FROM target is a bound parameter (a RecordId or array of them) and the SELECT list combines `*` with a subquery projection, only the subquery alias makes it into the response. All the row's regular fields are missing.

The shape works correctly when:
- FROM is a literal record id (not a parameter), or
- The SELECT list is `*` alone (no subquery), or
- The SELECT list has no `*` (only subquery aliases).

So the trigger is the specific combination: `SELECT *, <subquery> FROM $param`.

## Reproduction

```surql
CREATE thread:hello SET title = 'hi';
CREATE comment:c1 SET thread = thread:hello, text = 'first';

LET $id = thread:hello;

-- This works (literal id):
SELECT *, (SELECT * FROM comment WHERE thread = $parent.id) AS comments
  FROM thread:hello;
-- Returns: { id, title: 'hi', comments: [{...}] }

-- This drops the `*` fields (parameter):
SELECT *, (SELECT * FROM comment WHERE thread = $parent.id) AS comments
  FROM $id;
-- Returns: { comments: [{...}] }
-- ❌ no `title`, no `id`
```

## Workaround

In the sync-down fetch we used to do `SELECT *, (subquery) FROM $idsToFetch` to hydrate a row plus its joins in one round-trip. Since the `*` fields drop, the receiving client got a record with the subquery alias but no `title`, `published`, etc.

We split the work: fetch the bare row with `SELECT * FROM $idsToFetch` (no subquery), and let the in-browser SSP materialize the subqueries separately from the local DB.

**Files changed:**
- `packages/core/src/modules/sync/engine.ts` — comment at the `SELECT * FROM $idsToFetch` call points at this bug; do NOT add a subquery projection here.
