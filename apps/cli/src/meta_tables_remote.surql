-- ==================================================
-- SPOOKY INCANTATION
-- The Registry of active Live Queries (Incantations).
-- Scoped per session: `clientId` is set from `session::id()` server-side
-- in `fn::query::register`, so a record-token caller only sees the
-- incantations that originated from their current session. The SSP runs
-- as root and bypasses these table-level rules.
-- ==================================================

DEFINE TABLE OVERWRITE _00_query SCHEMALESS
    PERMISSIONS FOR select, create, update WHERE clientId = <string>session::id();

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
-- clients can never forge entries into their own view. Scoped by
-- `session::id()`: the SSP propagates the originating session's id from
-- _00_query.clientId onto each list-ref row, so a token-bearer only sees
-- its own list refs.
-- ==================================================

DEFINE TABLE OVERWRITE _00_list_ref SCHEMALESS
    PERMISSIONS FOR select WHERE clientId = <string>session::id();

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
-- SPOOKY CRDT + CURSOR
-- Per-field collaborative state (one row per record+field for CRDT,
-- one row per record+session+field for cursor). Records are array-keyed
-- (e.g. `_00_crdt:[thread:abc, "title"]`) so the client can UPSERT by
-- composite id without race-prone WHERE-based UPSERTs.
--
-- Reads inherit the parent's SELECT permission via `record_id.id != NONE`:
-- dereferencing a record runs under SELECT, so when the caller can't read
-- the parent row the dereference yields NONE and the meta row becomes
-- invisible too.
--
-- Writes inherit the parent's UPDATE permission via the {{CRDT_UPDATE_RULE}}
-- token below. The token is substituted at codegen time (schema_builder.rs)
-- with each CRDT-bearing parent's UPDATE expression — field references
-- rewritten to dereference through `record_id.<field>`. Permission rule
-- evaluation runs in elevated context, so an inline UPDATE check on the
-- parent doesn't actually enforce the parent's rule; only the dereference
-- pattern does.
--
-- No FOR delete: cleanup is server-side via the parent's DELETE event
-- (apps/cli/src/sp00ky.rs).
-- ==================================================

DEFINE TABLE OVERWRITE _00_crdt SCHEMAFULL
    PERMISSIONS
        FOR select WHERE record_id.id != NONE
        FOR create, update WHERE {{CRDT_UPDATE_RULE}};
DEFINE FIELD OVERWRITE record_id ON TABLE _00_crdt TYPE record;
DEFINE FIELD OVERWRITE field ON TABLE _00_crdt TYPE string;
DEFINE FIELD OVERWRITE state ON TABLE _00_crdt TYPE option<string>;
DEFINE INDEX OVERWRITE idx_crdt_record_field ON TABLE _00_crdt COLUMNS record_id, field UNIQUE;

DEFINE TABLE OVERWRITE _00_cursor SCHEMAFULL
    PERMISSIONS
        FOR select WHERE record_id.id != NONE
        FOR create, update WHERE {{CRDT_UPDATE_RULE}};
DEFINE FIELD OVERWRITE record_id ON TABLE _00_cursor TYPE record;
DEFINE FIELD OVERWRITE session_id ON TABLE _00_cursor TYPE string;
DEFINE FIELD OVERWRITE field ON TABLE _00_cursor TYPE string;
DEFINE FIELD OVERWRITE state ON TABLE _00_cursor TYPE option<string>;
DEFINE INDEX OVERWRITE idx_cursor_record_session_field ON TABLE _00_cursor COLUMNS record_id, session_id, field UNIQUE;
