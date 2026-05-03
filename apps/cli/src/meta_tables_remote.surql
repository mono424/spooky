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
-- Scoped per client: only the originating client sees its own list refs.
-- ==================================================

DEFINE TABLE OVERWRITE _00_list_ref SCHEMALESS
    PERMISSIONS FOR select, create, update WHERE clientId = $session.clientId;

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
-- Permission inherits from the linked record (same trick as _00_version).
-- ==================================================
DEFINE TABLE OVERWRITE _00_crdt SCHEMALESS
    PERMISSIONS FOR select, create, update, delete WHERE record_id.id != NONE;
DEFINE FIELD OVERWRITE record_id ON TABLE _00_crdt TYPE record;
DEFINE FIELD OVERWRITE field ON TABLE _00_crdt TYPE string;
DEFINE FIELD OVERWRITE state ON TABLE _00_crdt TYPE option<string>;
DEFINE INDEX OVERWRITE idx_record_field ON TABLE _00_crdt COLUMNS record_id, field UNIQUE;
