-- ==================================================
-- SINGLENODE / CLUSTER (HTTP) IMPLEMENTATION
-- ==================================================

DEFINE FUNCTION OVERWRITE fn::query::register($config: object) {
    -- clientId comes from session::id() (server-trusted); whatever the
    -- caller passed in $config.clientId is ignored. This is what the
    -- _00_query / _00_list_ref permission rules match against at read
    -- time, so the value must be derived inside the user's session.
    --
    -- $auth.id is injected server-side into params under `auth.id` so the
    -- SSP's permission rewrite (packages/ssp/src/policy.rs) can resolve
    -- `$auth.id` references in the registered table's SELECT predicate
    -- per-user. Without this, predicates that reference $auth fail closed
    -- (Predicate::False) at packages/ssp/src/policy.rs:184-194.
    LET $params_in = $config.params OR {};
    -- Inject both `$auth.id` and `$access` so the SSP's permission
    -- evaluator can substitute either when applying the table's SELECT
    -- predicate. Predicates like `published = true OR $access = "account"
    -- AND author.id = $auth.id` evaluate to "only published rows" if
    -- $access is missing — invisible-by-default for record-access users.
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
