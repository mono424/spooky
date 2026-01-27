-- ==================================================
-- SIDECAR (HTTP) IMPLEMENTATION
-- ==================================================

DEFINE FUNCTION fn::incantation::register($config: object) {
    IF $config.clientId == NONE THEN {
        THROW 'Missing client id';
    } END;

    LET $module_config = {
        id: <string>$config.id,
        sql: <string>$config.sql,
        params: <string>($config.params OR {}),
        clientId: <string>$config.clientId,
        ttl: <string>($config.ttl OR '1h'),
        lastActiveAt: <string>time::now(),
        format: <string>($config.format OR 'flat')
    };

    -- Check if Incantation already exists
    LET $inc_id = <record>('_spooky_incantation:' + <string>$config.id);
    LET $existing = SELECT * FROM $inc_id;

    IF array::len($existing) > 0 THEN {
        RETURN { hash: $existing[0].hash, array: $existing[0].array };
    } END;

    -- Call Sidecar via HTTP
    http::post('{{SIDECAR_ENDPOINT}}/view/register', $module_config, { "Authorization": "Bearer {{SIDECAR_SECRET}}" });
    
    -- Return empty placeholders
    RETURN { hash: "", array: [] };
};
