-- ==================================================
-- SPOOKY DATA HASH (Remote)
-- The "Shadow Graph" tracking the state of every record.
-- ==================================================

DEFINE TABLE _spooky_data_hash SCHEMAFULL
    PERMISSIONS FOR select, create, update WHERE true; -- In prod, you might restrict write access to server-side only

-- The actual record being tracked (e.g., comment:abc, thread:123)
DEFINE FIELD RecordId ON TABLE _spooky_data_hash TYPE record
    PERMISSIONS FOR select, create, update WHERE true;

-- H_intrinsic: BLAKE3 hash of the record's own scalar fields
DEFINE FIELD IntrinsicHash ON TABLE _spooky_data_hash TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

-- H_composition: XOR sum of all dependent children's TotalHashes
DEFINE FIELD CompositionHash ON TABLE _spooky_data_hash TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

-- H_total: Intrinsic XOR Composition
DEFINE FIELD TotalHash ON TABLE _spooky_data_hash TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE true;

-- Fast lookup by the original record ID
DEFINE INDEX idx_record_id ON TABLE _spooky_data_hash COLUMNS RecordId UNIQUE;
