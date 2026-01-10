-- ==================================================
-- SPOOKY SCHEMA
-- The provisioned schema state for the database. Currently only used in local cache.
-- Used in local-migrator.ts
-- ==================================================

DEFINE TABLE IF NOT EXISTS _spooky_schema SCHEMAFULL;
DEFINE FIELD IF NOT EXISTS id ON _spooky_schema TYPE string;
DEFINE FIELD IF NOT EXISTS hash ON _spooky_schema TYPE string;
DEFINE FIELD IF NOT EXISTS created_at ON _spooky_schema TYPE datetime VALUE time::now();
DEFINE INDEX IF NOT EXISTS unique_hash ON _spooky_schema FIELDS hash UNIQUE;

-- ==================================================
-- SPOOKY STREAM PROCESSOR STATE
-- Stores the local state of the stream processor (DBSP)
-- ==================================================

DEFINE TABLE IF NOT EXISTS _spooky_stream_processor_state SCHEMALESS
    PERMISSIONS FOR select, create, update, delete WHERE true;

DEFINE FIELD IF NOT EXISTS id ON _spooky_stream_processor_state TYPE string;
DEFINE FIELD IF NOT EXISTS state ON _spooky_stream_processor_state TYPE string;
DEFINE FIELD IF NOT EXISTS updated_at ON _spooky_stream_processor_state TYPE datetime VALUE time::now();

-- ==================================================
-- SPOOKY INCANTATION
-- The Registry of active Live Queries (Incantations).
-- ==================================================

DEFINE TABLE _spooky_incantation SCHEMALESS
    PERMISSIONS FOR select, create, update WHERE true;

-- The raw query string (for re-hydration/debugging)
DEFINE FIELD surrealQL ON TABLE _spooky_incantation TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE true;

-- The raw query string (for re-hydration/debugging)
DEFINE FIELD clientId ON TABLE _spooky_incantation TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE true;

-- The current XOR sum of all results in this query
DEFINE FIELD localHash ON TABLE _spooky_incantation TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE true;

-- The Radix Tree of Result IDs for efficient sync
DEFINE FIELD localTree ON TABLE _spooky_incantation TYPE any
    PERMISSIONS FOR select, create, update WHERE true;

-- The current XOR sum of all results in this query
DEFINE FIELD remoteHash ON TABLE _spooky_incantation TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE true;

-- The Radix Tree of Result IDs for efficient sync
DEFINE FIELD remoteTree ON TABLE _spooky_incantation TYPE any
    PERMISSIONS FOR select, create, update WHERE true;

-- For garbage collection (Heartbeat)
DEFINE FIELD lastActiveAt ON TABLE _spooky_incantation TYPE datetime DEFAULT time::now()
    PERMISSIONS FOR select, create, update WHERE true;

-- How long this Incantation stays alive without activity
DEFINE FIELD ttl ON TABLE _spooky_incantation TYPE duration
    PERMISSIONS FOR select, create, update WHERE true;

-- ==================================================
-- SPOOKY EVENTS
-- Stores create, update, and delete events
-- ==================================================

DEFINE TABLE _spooky_pending_mutations SCHEMAFULL
    PERMISSIONS FOR select, create, update, delete WHERE true;

DEFINE FIELD IF NOT EXISTS id ON _spooky_pending_mutations TYPE string;

DEFINE FIELD IF NOT EXISTS mutationType ON _spooky_pending_mutations TYPE string
    PERMISSIONS FOR select, create, update, delete WHERE true;

-- The target record ID (for update/delete) - maps to 'id' in the event object
DEFINE FIELD IF NOT EXISTS recordId ON _spooky_pending_mutations TYPE option<record>
    PERMISSIONS FOR select, create, update, delete WHERE true;

-- The data payload (for create/update)
DEFINE FIELD IF NOT EXISTS data ON _spooky_pending_mutations TYPE option<object> FLEXIBLE
    PERMISSIONS FOR select, create, update, delete WHERE true;
