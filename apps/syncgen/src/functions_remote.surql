-- ==================================================
-- FUNCTION: REGISTER INCANTATION (With Sorting)
-- Usage: fn::incantation::register({ ... sort_field: 'created_at' ... })
-- ==================================================
DEFINE FUNCTION fn::incantation::register($config: object) {
    -- UNPACK CONFIG
    LET $id         = $config.id;
    LET $client_id   = $config.client_id;
    LET $items      = $config.items;
    LET $paths      = $config.paths;
    LET $table      = $config.table;
    LET $filter     = $config.filter;
    LET $query      = $config.query; -- DBSP Plan (SQL String) REQUIRED
    
    -- Sorting Params (Default to created_at if missing)
    LET $sort_field = $config.sort_field OR 'created_at';
    LET $tree_init = {}; 
    
    -- 2. UPDATE INCANTATION REGISTRY
    -- Use String Cast for ID to avoid type::thing issues during schema load
    LET $rid_str = "_spooky_incantation:" + $id;
    LET $rid = <record>$rid_str;
    
    INSERT INTO _spooky_incantation {
        id:           $rid,
        hash:         NONE, -- Will be populated by DBSP
        clientId:     $client_id,
        surrealQL:    $query,
        tree:         <object>{},
        lastActiveAt: time::now(),
        ttl:          1h
    };

    -- 5. REGISTER WITH DBSP
    IF $query != NONE THEN {
        -- Call register_view(id, plan, params)
        -- Note: lib.rs handles persistence internally now.
        LET $dbsp_res = mod::dbsp::register_view($rid_str, $query, {});
        
        LET $init_res = $dbsp_res.result;
        IF $init_res != NONE THEN {
            -- Update using the explicit ID
            UPDATE $rid SET hash = $init_res.result_hash, tree = $init_res.tree;
        } END;
        
        -- DEBUG: Return the entire result
        RETURN $dbsp_res;
    } END;

    RETURN NONE;
};

DEFINE FUNCTION fn::incantation::heartbeat($id: record) {
    UPDATE $id SET lastActiveAt = time::now();
};