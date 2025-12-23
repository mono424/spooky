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
    
    -- NEW: Sorting Params (Default to created_at if missing)
    LET $sort_field = $config.sort_field OR 'created_at';
    
    -- 1. CALCULATE AGGREGATE HASH
    -- Improved: Use array aggregation instead of procedural loop to avoid init issues
    LET $hashes = (
        SELECT VALUE fn::spooky::view_hash(id, $paths) 
        FROM $items
    );
    LET $query_hash = mod::xor::blake3_xor_multi($hashes);

    -- 2. UPDATE INCANTATION REGISTRY
    UPSERT _spooky_incantation CONTENT {
        Id:           $id,
        Hash:         $query_hash,
        ClientId:     $client_id,
        LastActiveAt: time::now(),
        TTL:          1h
    };

    -- 3. REFRESH LOOKUPS
    DELETE _spooky_incantation_lookup WHERE IncantationId = $id;
    
    FOR $item IN $items {
        CREATE _spooky_incantation_lookup CONTENT {
            IncantationId: $id,
            Table:         $table,
            Where:         $filter,
            SortFields:    [$sort_field]
            SortDirections: ['asc']
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