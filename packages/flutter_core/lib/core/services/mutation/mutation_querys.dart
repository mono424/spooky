const String mutationCreateQuery = r'''
    BEGIN TRANSACTION;

    LET $created = CREATE type::table($table) CONTENT $data;
    LET $mutation = CREATE _spooky_pending_mutations SET
        mutation_type = 'create',
        record_id = $created.id,
        data = $data;

    RETURN {
        target: $created,
        mutation_id: $mutation.id
    };

    COMMIT TRANSACTION;
''';
