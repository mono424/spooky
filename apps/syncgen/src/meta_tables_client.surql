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
-- SPOOKY DATA HASH (Client)
-- Removed: Replaced by DBSP Module Internal Hashing
-- ==================================================

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
