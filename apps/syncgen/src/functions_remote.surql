-- ==================================================
-- FUNCTION: CALCULATE VIEW HASH
-- Usage: fn::spooky::view_hash(thread:123, ['comments'])
-- ==================================================
DEFINE FUNCTION fn::spooky::view_hash($rid: record, $paths: array<string>) {
    LET $shadow = (SELECT * FROM ONLY _spooky_data_hash WHERE RecordId = $rid);
    IF $shadow.RecordId == NONE { RETURN crypto::blake3(""); };

    -- 1. Start with the Intrinsic Hash
    LET $parts = [$shadow.IntrinsicHash];

    -- 2. Collect requested Composition Hashes (if they exist)
    -- We use a subquery to map the $paths array to values safely
    LET $extras = (
        SELECT VALUE $shadow.CompositionHash[$this] 
        FROM $paths 
        WHERE $shadow.CompositionHash[$this] != NONE
    );

    -- 3. Combine and Aggregate
    LET $all_parts = array::concat($parts, $extras);
    
    RETURN mod::xor::blake3_xor_multi($all_parts);
};

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
    LET $query      = $config.query; -- Optional DBSP Plan (SQL String)
    
    -- Sorting Params (Default to created_at if missing)
    LET $sort_field = $config.sort_field OR 'created_at';
    LET $tree_init = {}; 
    
    -- 1. CALCULATE AGGREGATE HASH
    LET $hashes = (
        SELECT VALUE fn::spooky::view_hash(id, $paths) 
        FROM $items
    );
    LET $query_hash = mod::xor::blake3_xor_multi($hashes);

    -- 2. UPDATE INCANTATION REGISTRY
    -- Use String Cast for ID to avoid type::thing issues during schema load
    LET $rid_str = "_spooky_incantation:" + $id;
    LET $rid = <record>$rid_str;
    
    INSERT INTO _spooky_incantation {
        id:           $rid,
        Id:           $id,
        Hash:         $query_hash,
        ClientId:     $client_id,
        SurrealQL:    $query,
        Tree:         <object>{},
        LastActiveAt: time::now(),
        TTL:          1h
    };



    -- 5. REGISTER WITH DBSP (If plan provided)
    IF $query != NONE THEN {
        LET $state = fn::dbsp::get_state();
        LET $dbsp_res = mod::dbsp::register_query($id, $query, $state);
        fn::dbsp::save_state($dbsp_res.new_state);
        
        LET $init_res = $dbsp_res.result;
        IF $init_res != NONE THEN {
            -- Update using the explicit ID
            UPDATE $rid SET Hash = $init_res.hash, Tree = $init_res.tree;
        } END;
        
        -- DEBUG: Return the entire result to inspect state
        RETURN $dbsp_res;
    } END;

    RETURN $query_hash;
};