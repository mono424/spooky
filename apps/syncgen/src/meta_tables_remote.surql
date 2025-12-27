-- ==================================================
-- SPOOKY DATA HASH (Remote)
-- The "Shadow Graph" tracking the state of every record.
-- ==================================================

DEFINE TABLE _spooky_data_hash SCHEMALESS
    PERMISSIONS FOR select, create, update WHERE true; -- In prod, you might restrict write access to server-side only

-- The actual record being tracked (e.g., comment:abc, thread:123)
DEFINE FIELD RecordId ON TABLE _spooky_data_hash TYPE record
    PERMISSIONS FOR select, create, update WHERE true;

-- H_intrinsic: The scalar hash of the current record (XOR of fields)
DEFINE FIELD IntrinsicHash ON TABLE _spooky_data_hash TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

-- H_composition: Object containing breakdown of dependent keys AND references (NO intrinsic scalar fields)
DEFINE FIELD CompositionHash ON TABLE _spooky_data_hash TYPE object
    PERMISSIONS FOR select, create, update WHERE true;

-- Explicitly define _xor for query parser
DEFINE FIELD CompositionHash._xor ON TABLE _spooky_data_hash TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

-- H_total: = CompositionHash._xor
DEFINE FIELD TotalHash ON TABLE _spooky_data_hash TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE true;

-- Fast lookup by the original record ID
DEFINE INDEX idx_record_id ON TABLE _spooky_data_hash COLUMNS RecordId UNIQUE;

-- Unregister from DBSP on deletion
DEFINE EVENT _spooky_dbsp_cleanup ON TABLE _spooky_incantation WHEN $event = "DELETE" THEN {
    let $result = mod::dbsp::unregister_query($before.Id);
};