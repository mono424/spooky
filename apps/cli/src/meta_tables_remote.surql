-- ==================================================
-- SPOOKY INCANTATION
-- The Registry of active Live Queries (Incantations).
-- ==================================================

DEFINE TABLE OVERWRITE _spooky_query SCHEMALESS
    PERMISSIONS FOR select, create, update WHERE true;

-- The raw query string (for re-hydration/debugging)
DEFINE FIELD OVERWRITE surql ON TABLE _spooky_query TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE true;

-- The client id
DEFINE FIELD OVERWRITE clientId ON TABLE _spooky_query TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE true;

-- For garbage collection (Heartbeat)
DEFINE FIELD OVERWRITE lastActiveAt ON TABLE _spooky_query TYPE datetime DEFAULT time::now()
    PERMISSIONS FOR select, create, update WHERE true;

-- How long this Incantation stays alive without activity
DEFINE FIELD OVERWRITE ttl ON TABLE _spooky_query TYPE duration
    PERMISSIONS FOR select, create, update WHERE true;

-- ==================================================
-- SPOOKY LIST REF
-- The current list of results for a query/incantation
-- ==================================================

DEFINE TABLE OVERWRITE _spooky_list_ref SCHEMALESS
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD OVERWRITE in ON TABLE _spooky_list_ref TYPE record
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD OVERWRITE out ON TABLE _spooky_list_ref TYPE record
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD OVERWRITE clientId ON TABLE _spooky_list_ref TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD OVERWRITE version ON TABLE _spooky_list_ref TYPE int
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD OVERWRITE updatedAt ON TABLE _spooky_list_ref TYPE datetime VALUE time::now() READONLY
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD OVERWRITE parent ON TABLE _spooky_list_ref TYPE option<record<_spooky_list_ref>>
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD OVERWRITE parent_rel ON TABLE _spooky_list_ref TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE true;

-- ==================================================
-- SPOOKY STREAM PROCESSOR STATE
-- Stores the local state of the stream processor (DBSP)
-- ==================================================

-- Remote-only meta tables (not synced to client)
DEFINE TABLE OVERWRITE _spooky_module_state SCHEMALESS PERMISSIONS FULL;

-- Unregister from DBSP on deletion
DEFINE EVENT OVERWRITE _spooky_dbsp_cleanup ON TABLE _spooky_query WHEN $event = "DELETE" THEN {
    let $result = mod::dbsp::unregister_view(<string>$before.id);
};

-- Note: Registration is now handled explicitly by fn::query::register calling mod::dbsp::register_view
-- The previous _spooky_dbsp_register event is removed to prevent recursion and double-registration.

-- ==================================================
-- SPOOKY VERSION
-- Tracks the version of every record for sync.
-- ==================================================
DEFINE TABLE OVERWRITE _spooky_version SCHEMALESS
    PERMISSIONS FOR select WHERE true;
DEFINE FIELD OVERWRITE record_id ON TABLE _spooky_version TYPE record;
DEFINE FIELD OVERWRITE version ON TABLE _spooky_version TYPE int;
DEFINE INDEX OVERWRITE idx_record_id ON TABLE _spooky_version COLUMNS record_id UNIQUE;
