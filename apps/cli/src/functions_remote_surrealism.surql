-- ==================================================
-- SURREALISM (WASM) IMPLEMENTATION
-- ==================================================

DEFINE FUNCTION OVERWRITE fn::query::register($config: object) {
    -- clientId comes from session::id() (server-trusted); whatever the
    -- caller passed in $config.clientId is ignored.
    -- Inject both `$auth.id` and `$access` so the DBSP module's permission
    -- evaluator can substitute either when applying the table's SELECT
    -- predicate. Predicates like `published = true OR $access = "account"
    -- AND author.id = $auth.id` evaluate to "only published rows" if
    -- $access is missing — invisible-by-default for record-access users.
    LET $params_in = $config.params OR {};
    LET $params_with_auth = object::extend($params_in, {
        auth: { id: <string>($auth.id OR '') },
        access: <string>($access OR '')
    });
    LET $module_config = {
        id: <string>$config.id,
        surql: <string>$config.surql,
        params: <string>$params_with_auth,
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
