# Unique index left partially populated after an interrupted `DEFINE INDEX OVERWRITE`

**Observed on:** SurrealDB server `v3.0.5` (RocksDB), whitepawn staging, 2026-09-06.

## Symptom

Two chat clients stop seeing each other's updates to an existing row while
inserts still arrive. Each client renders its own last write of the
`conversation` row (the DM list preview) and never the peer's, across live
updates and full reloads, while the `message` inserts show up on both sides
within a second. The database itself holds the correct row.

Mechanism: every `_00_<table>_mutation` event stamps the ingest payload with

```surql
_00_rv: (SELECT VALUE version FROM ONLY _00_version WHERE record_id = $after.id)
```

That lookup goes through the unique index `idx_record_id` on
`_00_version.record_id`. With the index entry missing the subquery returns
NONE, the SSP publishes the row at version 1 (`unwrap_or(1)` in the list_ref
edge), and every client already holding the row at version >= 1 keeps its
copy (`local < remote` is the only refetch trigger). The same missing entry
makes the event's `UPDATE _00_version SET version += 1 WHERE record_id = ...`
a no-op, so the stored version stops advancing too.

Measured on whitepawn staging: 965 of 2778 sampled rows (35%) across
`user`, `message`, `conversation`, `game`, `friendship`, `connection`,
`call`, `puzzle`, ... had no index entry while the `_00_version` row existed.
Misses were interleaved in time and in `_00_version` key order, so this is
not one interrupted pass but the residue of many.

## Reproduction

Against a database where the row exists (a table scan finds it) but the
index does not:

```surql
-- Index lookup: empty. EXPLAIN shows IndexScan on idx_record_id.
SELECT * FROM _00_version WHERE record_id = conversation:DM_3kdUk4Y9e;
-- Scan: finds it.
SELECT * FROM _00_version WITH NOINDEX WHERE record_id = conversation:DM_3kdUk4Y9e;
-- The index really has no entry: a duplicate CREATE is accepted.
RETURN { CREATE _00_version SET record_id = conversation:DM_3kdUk4Y9e, version = 999; THROW 'rollback'; };
INFO FOR INDEX idx_record_id ON TABLE _00_version;  -- { building: { status: 'ready' } }
```

How the entries went missing (best explanation, not reproduced in
isolation): `spky deploy` re-applies the internal schema whenever its bytes
change, and that schema said `DEFINE INDEX OVERWRITE idx_record_id ON TABLE
_00_version ...`. `OVERWRITE` rebuilds the index from scratch, synchronously,
inside the apply request, one entry per synced row: 737k on this database,
on a 1 vCPU container. That request regularly outlived the CLI's HTTP
timeout ("deploy hangs at Applying internal Sp00ky schema", the reason for
`SPKY_DB_HTTP_TIMEOUT_SECS=900`). A rebuild cut off part way leaves whatever
it had written; the deployment counter on this project was at 349.

## Workaround

1. Repair the data: `REBUILD INDEX idx_record_id ON TABLE _00_version;` then
   spot-check with the probe above. No duplicate `_00_version` rows were
   found in sampled tables, which a unique rebuild would reject.
2. `apps/cli/src/migrate.rs` (`prune_unchanged_index_defines`): the internal
   schema apply now reads `INFO FOR TABLE` and drops every `DEFINE INDEX`
   whose live definition already matches, so an unchanged index is never
   rebuilt by a deploy. A changed or missing index still gets its OVERWRITE.
3. `packages/ssp/src/circuit/circuit.rs` (`floor_row_version`, enabled
   server-side by `SspNode::apply_circuit_policy`): an ingested body whose
   `_00_rv` is missing or does not advance past the stored version is stamped
   `max(stored, highest version seen) + 1`, so subscribers refetch even when
   the DB-side stamp fails. Surfaced as the `ingest_rv_synthesized` counter
   and a rate-limited warning in the SSP log; a non-zero count is the signal
   to run the probe.

When SurrealDB rebuilds indexes robustly under interruption, (2) is still
worth keeping (a 737k-row rebuild per deploy is not free); (3) costs nothing.
