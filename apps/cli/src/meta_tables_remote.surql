-- ==================================================
-- SPOOKY INCANTATION
-- The Registry of active Live Queries (Incantations).
-- Scoped per client: a record-token caller only sees their own incantations.
-- The SSP itself runs as root and is unaffected by these table-level rules.
-- ==================================================

DEFINE TABLE OVERWRITE _00_query SCHEMALESS
    PERMISSIONS FOR select, create, update WHERE clientId = $session.clientId;

-- The raw query string (for re-hydration/debugging)
DEFINE FIELD OVERWRITE surql ON TABLE _00_query TYPE option<string>;

-- The client id
DEFINE FIELD OVERWRITE clientId ON TABLE _00_query TYPE option<string>;

-- For garbage collection (Heartbeat)
DEFINE FIELD OVERWRITE lastActiveAt ON TABLE _00_query TYPE datetime DEFAULT time::now();

-- How long this Incantation stays alive without activity
DEFINE FIELD OVERWRITE ttl ON TABLE _00_query TYPE duration;

-- ==================================================
-- SPOOKY LIST REF
-- The current list of results for a query/incantation.
-- Read-only to record-token callers — only the SSP (root) writes here, so
-- clients can never forge entries into their own view. Scoped per client so
-- a token-bearer only sees its own list refs.
-- ==================================================

DEFINE TABLE OVERWRITE _00_list_ref SCHEMALESS
    PERMISSIONS FOR select WHERE clientId = $session.clientId;

DEFINE FIELD OVERWRITE in ON TABLE _00_list_ref TYPE record;

DEFINE FIELD OVERWRITE out ON TABLE _00_list_ref TYPE record;

DEFINE FIELD OVERWRITE clientId ON TABLE _00_list_ref TYPE string;

DEFINE FIELD OVERWRITE version ON TABLE _00_list_ref TYPE int;

DEFINE FIELD OVERWRITE updatedAt ON TABLE _00_list_ref TYPE datetime VALUE time::now() READONLY;

DEFINE FIELD OVERWRITE parent ON TABLE _00_list_ref TYPE option<record<_00_list_ref>>;

DEFINE FIELD OVERWRITE parent_rel ON TABLE _00_list_ref TYPE option<string>;

-- ==================================================
-- SPOOKY STREAM PROCESSOR STATE
-- Stores the local state of the stream processor (DBSP)
-- ==================================================

-- Remote-only meta tables (not synced to client). Stays FULL because only the
-- root token (used by the SSP and the scheduler) ever reaches this table —
-- record-token callers can't address it.
DEFINE TABLE OVERWRITE _00_module_state SCHEMALESS PERMISSIONS FULL;

-- Unregister from DBSP on deletion
DEFINE EVENT OVERWRITE _00_dbsp_cleanup ON TABLE _00_query WHEN $event = "DELETE" THEN {
    let $result = mod::dbsp::unregister_view(<string>$before.id);
};

-- Note: Registration is now handled explicitly by fn::query::register calling mod::dbsp::register_view
-- The previous _00_dbsp_register event is removed to prevent recursion and double-registration.

-- ==================================================
-- SPOOKY VERSION
-- Tracks the version of every record for sync.
-- Permission inherits from the linked record: dereferencing record_id under
-- the caller's permission context returns NONE when the user can't read the
-- target row, which makes the version row invisible too.
-- ==================================================
DEFINE TABLE OVERWRITE _00_version SCHEMALESS
    PERMISSIONS FOR select WHERE record_id.id != NONE;
DEFINE FIELD OVERWRITE record_id ON TABLE _00_version TYPE record;
DEFINE FIELD OVERWRITE version ON TABLE _00_version TYPE int;
DEFINE INDEX OVERWRITE idx_record_id ON TABLE _00_version COLUMNS record_id UNIQUE;

-- ==================================================
-- SPOOKY CRDT
-- Stores per-field CRDT state for collaborative editing.
-- One row per record+field combination.
--
-- Reads inherit the parent's SELECT permission via record_id.id (record
-- dereferencing runs under SELECT). Writes inherit the parent's UPDATE
-- permission via fn::can_update_record below — `record_id.id != NONE`
-- alone is wrong for write actions because it only proves the caller can
-- read the parent, not edit it.
-- ==================================================

-- Returns true iff the caller has UPDATE permission on $r. Issues a no-op
-- UPDATE (SET _00_rv = _00_rv) on the parent; SurrealDB enforces the
-- parent's FOR update rule and returns [] when denied. _00_rv is injected
-- on every user table (apps/cli/src/main.rs:1233), so the assignment is
-- always valid.
--
-- Side effect: any DEFINE EVENT on the parent that fires WHEN $event = "UPDATE"
-- will run on every CRDT push. Gate downstream events with
-- WHEN $before._00_rv != $after._00_rv if that becomes a problem.
DEFINE FUNCTION OVERWRITE fn::can_update_record($r: record) -> bool {
    RETURN array::len((UPDATE $r SET _00_rv = _00_rv RETURN BEFORE)) > 0;
};

DEFINE TABLE OVERWRITE _00_crdt SCHEMALESS
    PERMISSIONS
        FOR select WHERE record_id.id != NONE
        FOR create, update WHERE fn::can_update_record(record_id);
-- No FOR delete: only the SSP cleans up CRDT rows (via the parent-record
-- DELETE event in apps/cli/src/sp00ky.rs:175, which runs server-side).
-- Record-token callers are denied delete by default; root bypasses.

DEFINE FIELD OVERWRITE record_id ON TABLE _00_crdt TYPE record;
DEFINE FIELD OVERWRITE field ON TABLE _00_crdt TYPE string;
DEFINE FIELD OVERWRITE state ON TABLE _00_crdt TYPE option<string>;
DEFINE INDEX OVERWRITE idx_record_field ON TABLE _00_crdt COLUMNS record_id, field UNIQUE;

-- ==================================================
-- SPOOKY CURSOR
-- Per-field ephemeral cursor / presence state for collaborative editing.
-- Lives in its own table so visibility can be tighter than _00_crdt:
-- only callers who can UPDATE the parent see (and push) cursors. Read-only
-- viewers don't get to watch other users' cursor positions.
--
-- Same shape as _00_crdt (record_id + field + opaque state blob). Pushed
-- by clients via INSERT ... ON DUPLICATE KEY UPDATE — see
-- packages/core/src/modules/crdt/crdt-field.ts::pushCursorState.
-- ==================================================
DEFINE TABLE OVERWRITE _00_cursor SCHEMALESS
    PERMISSIONS FOR select, create, update WHERE fn::can_update_record(record_id);
-- No FOR delete: cleanup is server-side via the parent-DELETE event in
-- apps/cli/src/sp00ky.rs. Record-token callers are denied delete by default.

DEFINE FIELD OVERWRITE record_id ON TABLE _00_cursor TYPE record;
DEFINE FIELD OVERWRITE field ON TABLE _00_cursor TYPE string;
DEFINE FIELD OVERWRITE state ON TABLE _00_cursor TYPE option<string>;
DEFINE INDEX OVERWRITE idx_cursor_record_field ON TABLE _00_cursor COLUMNS record_id, field UNIQUE;
