-- ==================================================
-- SINGLENODE / CLUSTER (HTTP) IMPLEMENTATION
-- ==================================================

DEFINE FUNCTION OVERWRITE fn::query::register($config: object) {
    -- clientId comes from session::id() (server-trusted); whatever the
    -- caller passed in $config.clientId is ignored. This is what the
    -- _00_query / _00_list_ref permission rules match against at read
    -- time, so the value must be derived inside the user's session.
    LET $module_config = {
        id: <string>$config.id,
        surql: <string>$config.surql,
        params: <string>($config.params OR {}),
        clientId: <string>session::id(),
        ttl: <string>($config.ttl OR '1h'),
        lastActiveAt: <string>time::now(),
        format: <string>($config.format OR 'flat')
    };

    -- Call SSP via HTTP
    http::post('{{ENDPOINT}}/view/register', $module_config, { "Authorization": "Bearer {{SECRET}}" });
};

DEFINE API OVERWRITE "/spooky"
    FOR get
    THEN {
        LET $info = http::get('{{ENDPOINT}}/info/text', {
            "Authorization": "Bearer {{SECRET}}"
        });
        RETURN {
            status: 200,
            body: <string>$info,
            headers: {
                'Content-Type': 'application/json'
            }
        };
    };
