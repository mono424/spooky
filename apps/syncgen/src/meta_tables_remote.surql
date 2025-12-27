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

-- ==================================================
-- SPOOKY MODULE STATE
-- Global state storage for the DBSP WASM module
-- ==================================================

DEFINE TABLE _spooky_module_state SCHEMALESS
    PERMISSIONS FOR select, create, update WHERE true;

-- The serialized WASM memory state (huge JSON object)
DEFINE FIELD State ON TABLE _spooky_module_state TYPE option<object>
    PERMISSIONS FOR select, create, update WHERE true;

-- Helper function to get or init state
DEFINE FUNCTION fn::dbsp::get_state() {
    RETURN (SELECT VALUE State FROM _spooky_module_state:dbsp_state)[0] OR {};
};

-- Helper function to save state
DEFINE FUNCTION fn::dbsp::save_state($new_state: object) {
    UPSERT _spooky_module_state:dbsp_state SET State = $new_state;
};

-- Unregister from DBSP on deletion
DEFINE EVENT _spooky_dbsp_cleanup ON TABLE _spooky_incantation WHEN $event = "DELETE" THEN {
    let $state = fn::dbsp::get_state();
    let $result = mod::dbsp::unregister_query($before.Id, $state);
    fn::dbsp::save_state($result.new_state);
};