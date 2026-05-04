-- ==================================================
-- SURREALISM (WASM) IMPLEMENTATION
-- ==================================================

DEFINE FUNCTION OVERWRITE fn::query::register($config: object) {
    -- clientId comes from session::id() (server-trusted); whatever the
    -- caller passed in $config.clientId is ignored.
    LET $module_config = {
        id: <string>$config.id,
        surql: <string>$config.surql,
        params: <string>($config.params OR {}),
        clientId: <string>session::id(),
        ttl: <string>($config.ttl OR '1h'),
        lastActiveAt: <string>time::now()
    };

    -- Call DBSP module directly
    mod::dbsp::register_view($module_config);
};

DEFINE API OVERWRITE "/spooky"
    FOR get
    THEN {
        RETURN {
            status: 200,
            body: {
                type: "surrealism",
                status: "ready",
                scheduler: false,
                ssp_instances: 0
            }
        };
    };
