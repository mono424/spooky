-- ==================================================
-- SIDECAR (HTTP) IMPLEMENTATION
-- ==================================================

DEFINE FUNCTION fn::incantation::register($config: object) {
    IF $config.clientId == NONE THEN {
        THROW 'Missing client id';
    } END;

    LET $module_config = {
        id: <string>$config.id,
        surrealQL: <string>$config.surrealQL,
        params: <string>($config.params OR {}),
        clientId: <string>$config.clientId,
        ttl: <string>($config.ttl OR '1h'),
        lastActiveAt: <string>time::now()
    };

    -- Check if Incantation already exists
    LET $existing = SELECT * FROM type::thing('_spooky_incantation', $config.id);

    IF array::len($existing) > 0 THEN {
        RETURN { hash: $existing[0].hash, tree: $existing[0].tree };
    } END;

    -- Call Sidecar via HTTP
    http::post('{{SIDECAR_ENDPOINT}}/view/register', $module_config, { "Authorization": "Bearer {{SIDECAR_SECRET}}" });
    
    -- Return empty placeholders
    RETURN { hash: "", tree: NONE };
};
