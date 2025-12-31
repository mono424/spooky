-- ==================================================
-- FUNCTION: REGISTER INCANTATION (With Sorting)
-- Usage: fn::incantation::register({ ... sort_field: 'created_at' ... })
-- ==================================================
DEFINE FUNCTION fn::incantation::register($config: object) {
    IF $config.clientId == NONE THEN {
        THROW 'Missing client id';
    } END;

    -- Prepare config for DBSP module
    -- Enforce server-side client_id and timestamps
    LET $module_config = {
        id: <string>$config.id,
        surrealQL: <string>$config.surrealQL,
        params: <string>($config.params OR {}),
        clientId: <string>$config.clientId,
        ttl: <string>($config.ttl OR '1h'),
        lastActiveAt: <string>time::now()
    };
    
    -- Call DBSP to register and persist
    -- Returns { hash: "...", tree: ... }
    RETURN mod::dbsp::register_view($module_config);
};

DEFINE FUNCTION fn::incantation::heartbeat($id: record) {
    UPDATE $id SET lastActiveAt = time::now();
};