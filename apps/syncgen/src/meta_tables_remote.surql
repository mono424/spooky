-- Remote-only meta tables (not synced to client)
DEFINE TABLE _spooky_module_state SCHEMALESS PERMISSIONS FULL;

-- Unregister from DBSP on deletion
DEFINE EVENT _spooky_dbsp_cleanup ON TABLE _spooky_incantation WHEN $event = "DELETE" THEN {
    let $result = mod::dbsp::unregister_query(<string>$before.id);
};

-- Register with DBSP on creation or update
-- Register with DBSP on creation or update
DEFINE EVENT _spooky_dbsp_register ON TABLE _spooky_incantation WHEN $event = "CREATE" OR ($event = "UPDATE" AND ($after.surrealQL != $before.surrealQL OR $after.params != $before.params)) THEN {
    LET $res = mod::dbsp::register_view(<string>$after.id, $after.surrealQL, <string>($after.params OR {}));
    -- Update the incantation with the initial hash computed by DBSP
    IF $res.result_hash != NONE THEN {
        UPDATE _spooky_incantation SET hash = $res.result_hash, tree = $res.tree WHERE id = $after.id; 
    } END;
};