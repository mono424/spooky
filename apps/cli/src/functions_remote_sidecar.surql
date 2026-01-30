-- ==================================================
-- SIDECAR (HTTP) IMPLEMENTATION
-- ==================================================

DEFINE FUNCTION fn::query::register($config: object) {
    IF $config.clientId == NONE THEN {
        THROW 'Missing client id';
    } END;

    LET $module_config = {
        id: <string>$config.id,
        surql: <string>$config.surql,
        params: <string>($config.params OR {}),
        clientId: <string>$config.clientId,
        ttl: <string>($config.ttl OR '1h'),
        lastActiveAt: <string>time::now(),
        format: <string>($config.format OR 'flat')
    };

    -- Call Sidecar via HTTP
    http::post('{{SIDECAR_ENDPOINT}}/view/register', $module_config, { "Authorization": "Bearer {{SIDECAR_SECRET}}" });
};
