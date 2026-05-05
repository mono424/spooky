-- ==================================================
-- CLIENT-SIDE META TABLES
--
-- These tables live in the per-device local SurrealKV cache, which is
-- single-tenant by construction (one user, one device). The codegen step in
-- apps/cli/src/codegen.rs rewrites every PERMISSIONS clause to `WHERE true`
-- when emitting the client schema, because there's no $session / $auth on
-- the local DB to scope against. For multi-tenant scoping, see the
-- corresponding rules in meta_tables_remote.surql which run on the server.
-- ==================================================

-- ==================================================
-- SPOOKY SCHEMA
-- The provisioned schema state for the database. Currently only used in local cache.
-- Used in local-migrator.ts
-- ==================================================

DEFINE TABLE IF NOT EXISTS _00_schema SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS id ON _00_schema TYPE string;
DEFINE FIELD IF NOT EXISTS hash ON _00_schema TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON _00_schema TYPE datetime VALUE time::now();
DEFINE INDEX IF NOT EXISTS unique_hash ON _00_schema FIELDS hash UNIQUE;

-- ==================================================
-- SPOOKY STREAM PROCESSOR STATE
-- Stores the local state of the stream processor (DBSP)
-- ==================================================

DEFINE TABLE IF NOT EXISTS _00_stream_processor_state SCHEMALESS
    PERMISSIONS FOR select, create, update, delete WHERE true;

DEFINE FIELD IF NOT EXISTS id ON _00_stream_processor_state TYPE string;
DEFINE FIELD IF NOT EXISTS state ON _00_stream_processor_state TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON _00_stream_processor_state TYPE datetime VALUE time::now();

-- ==================================================
-- SPOOKY INCANTATION
-- The Registry of active Live Queries (Incantations).
-- ==================================================

DEFINE TABLE _00_query SCHEMALESS
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD surql ON TABLE _00_query TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD localArray ON TABLE _00_query TYPE any
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD remoteArray ON TABLE _00_query TYPE any
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD lastActiveAt ON TABLE _00_query TYPE datetime DEFAULT time::now()
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD ttl ON TABLE _00_query TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD tableName ON TABLE _00_query TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

-- Metrics: lifecycle and per-query observability. Rolling-window percentiles
-- are computed in the client (DataModule) from the last N materialization
-- samples and persisted on each ingest so devtools can read them without
-- re-running the query.

DEFINE FIELD createdAt ON TABLE _00_query TYPE datetime DEFAULT time::now() READONLY
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD registrationTime ON TABLE _00_query TYPE option<float>
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD materializationP55 ON TABLE _00_query TYPE option<float>
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD materializationP90 ON TABLE _00_query TYPE option<float>
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD materializationP99 ON TABLE _00_query TYPE option<float>
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD lastIngestLatency ON TABLE _00_query TYPE option<float>
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD updateCount ON TABLE _00_query TYPE int DEFAULT 0
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD rowCount ON TABLE _00_query TYPE int DEFAULT 0
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD errorCount ON TABLE _00_query TYPE int DEFAULT 0
    PERMISSIONS FOR select, create, update WHERE true;

-- ==================================================
-- SPOOKY EVENTS
-- Stores create, update, and delete events
-- ==================================================

DEFINE TABLE _00_pending_mutations SCHEMAFULL
    PERMISSIONS FOR select, create, update, delete WHERE true;

DEFINE FIELD IF NOT EXISTS id ON _00_pending_mutations TYPE string;

DEFINE FIELD IF NOT EXISTS mutationType ON _00_pending_mutations TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

-- The target record ID (for update/delete) - maps to 'id' in the event object
DEFINE FIELD IF NOT EXISTS recordId ON _00_pending_mutations TYPE option<record>
    PERMISSIONS FOR select, create, update WHERE true;

-- The data payload (for create/update)
DEFINE FIELD IF NOT EXISTS data ON _00_pending_mutations TYPE option<object> FLEXIBLE
    PERMISSIONS FOR select, create, update WHERE true;

-- CRDT state lives inline on the parent record itself (the `@crdt`-
-- annotated field). `@crdt`-only fields hold the base64 LoroDoc
-- snapshot directly; `@crdt @cursor` fields hold an
-- `option<object> FLEXIBLE` of shape `{ state, cursors }` (rewritten
-- by the schema-builder). Same shape on client and server.
