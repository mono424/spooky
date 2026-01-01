-- ==================================================
-- SURREALISM (WASM) IMPLEMENTATION
-- ==================================================

DEFINE FUNCTION fn::incantation::register($config: object) {
    IF $config.clientId == NONE THEN {
        THROW 'Missing client id';
    } END;

    -- Prepare config for DBSP module
    LET $module_config = {
        id: <string>$config.id,
        surrealQL: <string>$config.surrealQL,
        params: <string>($config.params OR {}),
        clientId: <string>$config.clientId,
        ttl: <string>($config.ttl OR '1h'),
        lastActiveAt: <string>time::now()
    };
    
    -- Call DBSP module directly
    mod::dbsp::register_view($module_config);
    RETURN NONE;
};
