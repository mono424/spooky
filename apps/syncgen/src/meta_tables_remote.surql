-- Remote-only meta tables (not synced to client)
DEFINE TABLE _spooky_module_state SCHEMALESS PERMISSIONS FULL;

-- Unregister from DBSP on deletion
DEFINE EVENT _spooky_dbsp_cleanup ON TABLE _spooky_incantation WHEN $event = "DELETE" THEN {
    let $result = mod::dbsp::unregister_query($before.Id);
};