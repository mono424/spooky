-- ==================================================
-- AUTO-GENERATED SPOOKY EVENTS
-- ==================================================

-- Table: user Mutation
DEFINE EVENT OVERWRITE _spooky_user_mutation ON TABLE user
WHEN $before != $after
THEN {
    LET $new_intrinsic = crypto::blake3(<string>{
        id: $after.id,
        username: $after.username,
        password: $after.password,
        created_at: $after.created_at
    });

    LET $old_hash_data = (SELECT * FROM _spooky_data_hash WHERE RecordId = $before.id)[0];
    LET $composition = $old_hash_data.CompositionHash OR "";
    LET $old_total = $old_hash_data.TotalHash OR "";

    LET $new_total = crypto::blake3($new_intrinsic + $composition);

    UPSERT _spooky_data_hash CONTENT {
        RecordId: $after.id,
        IntrinsicHash: $new_intrinsic,
        CompositionHash: $composition,
        TotalHash: $new_total
    };

    -- Recalculate Composition (Bubble Up)
    LET $children_hashes = [];
    LET $ch_thread = (SELECT TotalHash FROM _spooky_data_hash WHERE RecordId IN (SELECT id FROM thread WHERE author = $after.id)).TotalHash;
    $children_hashes = array::union($children_hashes, $ch_thread);
    LET $ch_comment = (SELECT TotalHash FROM _spooky_data_hash WHERE RecordId IN (SELECT id FROM comment WHERE author = $after.id)).TotalHash;
    $children_hashes = array::union($children_hashes, $ch_comment);
    LET $new_composition = crypto::blake3($children_hashes);
    LET $composition = $new_composition;
    LET $new_total = crypto::blake3($new_intrinsic + $composition);
    UPSERT _spooky_data_hash CONTENT {
        RecordId: $after.id,
        IntrinsicHash: $new_intrinsic,
        CompositionHash: $composition,
        TotalHash: $new_total
    };
};

-- Table: user Deletion
DEFINE EVENT OVERWRITE _spooky_user_delete ON TABLE user
WHEN $event = "DELETE"
THEN {
    DELETE _spooky_data_hash WHERE RecordId = $before.id;
};

-- Table: thread Mutation
DEFINE EVENT OVERWRITE _spooky_thread_mutation ON TABLE thread
WHEN $before != $after
THEN {
    LET $new_intrinsic = crypto::blake3(<string>{
        id: $after.id,
        title: $after.title,
        content: $after.content,
        created_at: $after.created_at
    });

    LET $old_hash_data = (SELECT * FROM _spooky_data_hash WHERE RecordId = $before.id)[0];
    LET $composition = $old_hash_data.CompositionHash OR "";
    LET $old_total = $old_hash_data.TotalHash OR "";

    LET $new_total = crypto::blake3($new_intrinsic + $composition);

    UPSERT _spooky_data_hash CONTENT {
        RecordId: $after.id,
        IntrinsicHash: $new_intrinsic,
        CompositionHash: $composition,
        TotalHash: $new_total
    };

    -- Recalculate Composition (Bubble Up)
    LET $children_hashes = [];
    LET $ch_comment = (SELECT TotalHash FROM _spooky_data_hash WHERE RecordId IN (SELECT id FROM comment WHERE thread = $after.id)).TotalHash;
    $children_hashes = array::union($children_hashes, $ch_comment);
    LET $new_composition = crypto::blake3($children_hashes);
    LET $composition = $new_composition;
    LET $new_total = crypto::blake3($new_intrinsic + $composition);
    UPSERT _spooky_data_hash CONTENT {
        RecordId: $after.id,
        IntrinsicHash: $new_intrinsic,
        CompositionHash: $composition,
        TotalHash: $new_total
    };
    -- Trigger Parent Update (user)
    UPDATE $after.author SET _updated_at = time::now(); -- Try to look for typical timestamp or strict schema might fail
};

-- Table: thread Deletion
DEFINE EVENT OVERWRITE _spooky_thread_delete ON TABLE thread
WHEN $event = "DELETE"
THEN {
    DELETE _spooky_data_hash WHERE RecordId = $before.id;
    UPDATE $before.author SET _updated_at = time::now();
};

-- Table: comment Mutation
DEFINE EVENT OVERWRITE _spooky_comment_mutation ON TABLE comment
WHEN $before != $after
THEN {
    LET $new_intrinsic = crypto::blake3(<string>{
        id: $after.id,
        content: $after.content,
        created_at: $after.created_at
    });

    LET $old_hash_data = (SELECT * FROM _spooky_data_hash WHERE RecordId = $before.id)[0];
    LET $composition = $old_hash_data.CompositionHash OR "";
    LET $old_total = $old_hash_data.TotalHash OR "";

    LET $new_total = crypto::blake3($new_intrinsic + $composition);

    UPSERT _spooky_data_hash CONTENT {
        RecordId: $after.id,
        IntrinsicHash: $new_intrinsic,
        CompositionHash: $composition,
        TotalHash: $new_total
    };

    -- Trigger Parent Update (thread)
    UPDATE $after.thread SET _updated_at = time::now(); -- Try to look for typical timestamp or strict schema might fail
    -- Trigger Parent Update (user)
    UPDATE $after.author SET _updated_at = time::now(); -- Try to look for typical timestamp or strict schema might fail
};

-- Table: comment Deletion
DEFINE EVENT OVERWRITE _spooky_comment_delete ON TABLE comment
WHEN $event = "DELETE"
THEN {
    DELETE _spooky_data_hash WHERE RecordId = $before.id;
    UPDATE $before.thread SET _updated_at = time::now();
    UPDATE $before.author SET _updated_at = time::now();
};

-- Table: commented_on Mutation
DEFINE EVENT OVERWRITE _spooky_commented_on_mutation ON TABLE commented_on
WHEN $before != $after
THEN {
    LET $new_intrinsic = crypto::blake3(<string>{
        id: $after.id
    });

    LET $old_hash_data = (SELECT * FROM _spooky_data_hash WHERE RecordId = $before.id)[0];
    LET $composition = $old_hash_data.CompositionHash OR "";
    LET $old_total = $old_hash_data.TotalHash OR "";

    LET $new_total = crypto::blake3($new_intrinsic + $composition);

    UPSERT _spooky_data_hash CONTENT {
        RecordId: $after.id,
        IntrinsicHash: $new_intrinsic,
        CompositionHash: $composition,
        TotalHash: $new_total
    };

};

-- Table: commented_on Deletion
DEFINE EVENT OVERWRITE _spooky_commented_on_delete ON TABLE commented_on
WHEN $event = "DELETE"
THEN {
    DELETE _spooky_data_hash WHERE RecordId = $before.id;
};



