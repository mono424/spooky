-- Migration: up
-- Created: 2026-05-04T09:29:03+00:00
--
-- Move CRDT and cursor state out of embedded `_00_crdt` / `_00_cursor`
-- FLEXIBLE object fields on parent records and into dedicated tables.
-- Splitting the state off the parent row is what makes offline edits
-- mergeable: each (record, field) gets its own row, so concurrent offline
-- writes don't collide on the parent's last-write-wins semantics.
--
-- Records are array-keyed (e.g. `_00_crdt:[thread:abc, "title"]`) so the
-- client can UPSERT by composite id without race-prone WHERE-based UPSERTs
-- (UPSERT … WHERE silently fails to create rows when no match exists).

-- Removed: embedded fields on each CRDT-bearing parent.
REMOVE FIELD IF EXISTS _00_crdt   ON TABLE thread;
REMOVE FIELD IF EXISTS _00_cursor ON TABLE thread;

-- The earlier design used `fn::can_update_record` to gate writes; that
-- approach doesn't work because permission expressions and DEFINE FUNCTION
-- bodies run in elevated context, so the no-op UPDATE inside doesn't
-- enforce the parent's UPDATE rule. Drop it.
REMOVE FUNCTION IF EXISTS fn::can_update_record;

-- Per-field CRDT state. Reads inherit the parent's SELECT rule via
-- `record_id` dereference (denied refs become NONE, hiding the meta row).
-- Writes inherit the parent's UPDATE rule via the dereferenced expression
-- below — this rule is generated from the user schema's parent-table
-- PERMISSIONS clause by `apps/cli/src/schema_builder.rs`.
DEFINE TABLE OVERWRITE _00_crdt SCHEMAFULL
    PERMISSIONS
        FOR select WHERE record_id.id != NONE
        FOR create, update WHERE
            (record_id.tb() = "thread"
             AND ($access = 'account'
                  AND (record_id.author.id = $auth.id
                       OR $auth.id INSIDE (
                           SELECT VALUE in FROM collaborates_on WHERE out = record_id
                       ))));
DEFINE FIELD OVERWRITE record_id ON TABLE _00_crdt TYPE record;
DEFINE FIELD OVERWRITE field     ON TABLE _00_crdt TYPE string;
DEFINE FIELD OVERWRITE state     ON TABLE _00_crdt TYPE option<string>;
DEFINE INDEX OVERWRITE idx_crdt_record_field ON TABLE _00_crdt COLUMNS record_id, field UNIQUE;

-- Per-(record, session, field) cursor presence. Same permission scheme.
DEFINE TABLE OVERWRITE _00_cursor SCHEMAFULL
    PERMISSIONS
        FOR select WHERE record_id.id != NONE
        FOR create, update WHERE
            (record_id.tb() = "thread"
             AND ($access = 'account'
                  AND (record_id.author.id = $auth.id
                       OR $auth.id INSIDE (
                           SELECT VALUE in FROM collaborates_on WHERE out = record_id
                       ))));
DEFINE FIELD OVERWRITE record_id  ON TABLE _00_cursor TYPE record;
DEFINE FIELD OVERWRITE session_id ON TABLE _00_cursor TYPE string;
DEFINE FIELD OVERWRITE field      ON TABLE _00_cursor TYPE string;
DEFINE FIELD OVERWRITE state      ON TABLE _00_cursor TYPE option<string>;
DEFINE INDEX OVERWRITE idx_cursor_record_session_field ON TABLE _00_cursor COLUMNS record_id, session_id, field UNIQUE;
