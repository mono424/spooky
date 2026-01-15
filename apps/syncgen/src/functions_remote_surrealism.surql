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
    
    -- Check if Incantation already exists
    LET $inc_id = <record>('_spooky_incantation:' + <string>$config.id);
    LET $existing = SELECT * FROM $inc_id;

    IF array::len($existing) > 0 THEN {
        RETURN { hash: $existing[0].hash, array: $existing[0].array };
    } END;

    -- Call DBSP module directly
    mod::dbsp::register_view($module_config);

    -- Return empty placeholders
    RETURN { hash: "", array: [] };
};
