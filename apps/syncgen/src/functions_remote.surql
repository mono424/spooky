-- ==================================================
-- FUNCTION: CALCULATE VIEW HASH
-- Usage: fn::spooky::view_hash(thread:123, ['comments'])
-- ==================================================
DEFINE FUNCTION fn::spooky::view_hash($rid: record, $paths: array<string>) {
    -- 1. Fetch the Shadow Record
    LET $shadow = (SELECT * FROM ONLY _spooky_data_hash WHERE RecordId = $rid);

    -- 2. Guard: If no shadow record exists, return generic empty hash (or handle error)
    IF $shadow.RecordId == NONE THEN {
        RETURN crypto::blake3("");
    } END;

    -- 3. Base: The Intrinsic Hash (Scalar State) is ALWAYS included
    LET $final_hash = $shadow.IntrinsicHash;

    -- 4. Composition: Selectively Mix in Dependencies
    -- We iterate over the requested paths (e.g. 'comments', 'author')
    FOR $path IN $paths {
        -- Check if the CompositionHash object has this key
        LET $part_hash = $shadow.CompositionHashes[$path];
        
        -- If found, XOR it into the state
        IF $part_hash != NONE THEN {
            $final_hash = mod::xor::blake3_xor($final_hash, $part_hash);
        } END;
    };

    RETURN $final_hash;
};

-- ==================================================
-- FUNCTION: REGISTER INCANTATION (With Sorting)
-- Usage: fn::incantation::register({ ... sort_field: 'created_at' ... })
-- ==================================================
DEFINE FUNCTION fn::incantation::register($config: object) {
    -- UNPACK CONFIG
    LET $id         = $config.id;
    LET $items      = $config.items;
    LET $paths      = $config.paths;
    LET $table      = $config.table;
    LET $filter     = $config.filter;
    
    -- NEW: Sorting Params (Default to created_at if missing)
    LET $sort_field = $config.sort_field OR 'created_at';
    
    -- 1. CALCULATE AGGREGATE HASH
    LET $query_hash = "";
    FOR $item IN $items {
        LET $h = fn::spooky::view_hash($item.id, $paths);
        $query_hash = mod::xor::blake3_xor($query_hash, $h);
    };

    -- 2. UPDATE INCANTATION REGISTRY
    UPSERT _spooky_incantation CONTENT {
        Id:           $id,
        Hash:         $query_hash,
        LastActiveAt: time::now(),
        TTL:          1h
    };

    -- 3. REFRESH LOOKUPS
    DELETE _spooky_incantation_lookup WHERE IncantationId = $id;
    
    FOR $item IN $items {
        CREATE _spooky_incantation_lookup CONTENT {
            IncantationId: $id,
            Table:         $table,
            RecordId:      $item.id,
            Where:         $filter
        };
    };

    -- 4. HANDLE TAIL (Dynamic Cursor)
    IF array::len($items) > 0 THEN {
        LET $last = array::last($items);
        LET $tail_val = $last[$sort_field];

        UPSERT _spooky_incantation_tail CONTENT {
            IncantationId: $id,
            SortField:     $sort_field, 
            TailValues:    [$tail_val] 
        };
    } ELSE {
        DELETE _spooky_incantation_tail WHERE IncantationId = $id;
    } END;

    RETURN $query_hash;
};