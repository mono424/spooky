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

-- The auth user id that registered this query. The SSP propagates it
-- onto each `_00_list_ref` row so live-query delivery can be gated on
-- the user (stable across re-auth / reconnect) instead of the per-
-- connection `session::id()`. Empty string means "no auth recorded",
-- which is fine for SSP-only registrations and unreadable to any
-- real authenticated user via the list_ref permission below.
DEFINE FIELD OVERWRITE auth_id ON TABLE _00_query TYPE string DEFAULT '';

-- For garbage collection (Heartbeat)
DEFINE FIELD OVERWRITE lastActiveAt ON TABLE _00_query TYPE datetime DEFAULT time::now();

-- How long this Incantation stays alive without activity
DEFINE FIELD OVERWRITE ttl ON TABLE _00_query TYPE duration;

-- Metrics: lifecycle and per-query observability. The SSP keeps a rolling
-- window of materialization samples per view and writes the persisted
-- percentiles on each ingest, so consumers can read them without
-- re-running the query.
DEFINE FIELD OVERWRITE createdAt ON TABLE _00_query TYPE datetime DEFAULT time::now() READONLY;
DEFINE FIELD OVERWRITE registrationTime ON TABLE _00_query TYPE option<float>;
DEFINE FIELD OVERWRITE materializationP55 ON TABLE _00_query TYPE option<float>;
DEFINE FIELD OVERWRITE materializationP90 ON TABLE _00_query TYPE option<float>;
DEFINE FIELD OVERWRITE materializationP99 ON TABLE _00_query TYPE option<float>;
DEFINE FIELD OVERWRITE lastIngestLatency ON TABLE _00_query TYPE option<float>;
DEFINE FIELD OVERWRITE updateCount ON TABLE _00_query TYPE int DEFAULT 0;
DEFINE FIELD OVERWRITE rowCount ON TABLE _00_query TYPE int DEFAULT 0;
DEFINE FIELD OVERWRITE errorCount ON TABLE _00_query TYPE int DEFAULT 0;

-- ==================================================
-- SPOOKY LIST REF
-- The current list of results for a query/incantation.
-- Read-only to record-token callers — only the SSP (root) writes here, so
-- clients can never forge entries into their own view. Scoped by
-- `session::id()`: the SSP propagates the originating session's id from
-- _00_query.clientId onto each list-ref row, so a token-bearer only sees
-- its own list refs.
-- ==================================================

-- Scoped per user, not per session. The originating session's
-- `auth.id` is copied here from `_00_query.auth_id` at write time, so
-- every session of the same user receives LIVE updates for queries
-- that user has registered, regardless of which connection wrote
-- them. The client still filters in-memory by registered queryId
-- (see `Sp00kySync.handleRemoteListRefChange`), so cross-session
-- entries it doesn't care about are dropped after delivery.
DEFINE TABLE OVERWRITE _00_list_ref SCHEMALESS
    PERMISSIONS FOR select WHERE auth_id = <string>$auth.id;

DEFINE FIELD OVERWRITE in ON TABLE _00_list_ref TYPE record;

DEFINE FIELD OVERWRITE out ON TABLE _00_list_ref TYPE record;

DEFINE FIELD OVERWRITE clientId ON TABLE _00_list_ref TYPE string;

DEFINE FIELD OVERWRITE auth_id ON TABLE _00_list_ref TYPE string DEFAULT '';

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

-- CRDT and cursor state live inline on the parent record itself —
-- `@crdt`-only fields hold a base64 LoroDoc snapshot directly; `@crdt
-- @cursor` fields hold an `option<object> FLEXIBLE` of shape `{ state,
-- cursors }` (the schema-builder rewrites the user's TYPE clause).
-- No sidecar tables. Sync-down naturally carries CRDT state with the
-- parent row; cross-browser delivery rides the parent's existing LIVE
-- feed without a follow-up subquery.
