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
-- The "Shadow Graph" tracking the state of every record.
-- ==================================================

DEFINE TABLE _spooky_data_hash SCHEMAFULL
    PERMISSIONS FOR select, create, update WHERE true; -- In prod, you might restrict write access to server-side only

-- The actual record being tracked (e.g., comment:abc, thread:123)
DEFINE FIELD recordId ON TABLE _spooky_data_hash TYPE record
    PERMISSIONS FOR select, create, update WHERE true;

-- H_intrinsic: BLAKE3 hash of the record's own scalar fields
DEFINE FIELD intrinsicHash ON TABLE _spooky_data_hash TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

-- H_composition: XOR sum of all dependent children's TotalHashes
DEFINE FIELD compositionHash ON TABLE _spooky_data_hash TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

-- H_total: Intrinsic XOR Composition
DEFINE FIELD totalHash ON TABLE _spooky_data_hash TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE true;

-- CLIENT-SPECIFIC FIELDS
DEFINE FIELD isDirty ON TABLE _spooky_data_hash TYPE bool DEFAULT false
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD pendingDelete ON TABLE _spooky_data_hash TYPE bool DEFAULT false
    PERMISSIONS FOR select, create, update WHERE true;

-- Fast lookup by the original record ID
DEFINE INDEX idx_record_id ON TABLE _spooky_data_hash COLUMNS recordId UNIQUE;

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
