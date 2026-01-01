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

    -- Call Sidecar via HTTP
    RETURN http::post('{{SIDECAR_ENDPOINT}}/register_view', $module_config, { "Authorization": "Bearer {{SIDECAR_SECRET}}" });
};
