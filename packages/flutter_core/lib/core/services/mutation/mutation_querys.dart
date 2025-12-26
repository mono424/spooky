class MutationTargetResponse {
  final String id;
  final Map<String, dynamic> record;

  MutationTargetResponse({required this.id, required this.record});

  factory MutationTargetResponse.fromJason(Map<String, dynamic> json) {
    return MutationTargetResponse(id: json['id'] as String, record: json);
  }
}

class MutationResponse {
  final String mutationID;
  final MutationTargetResponse target;

  MutationResponse({required this.mutationID, required this.target});

  factory MutationResponse.fromJson(Map<String, dynamic> json) {
    return MutationResponse(
      mutationID: json['mutation_id'] as String,
      target: MutationTargetResponse.fromJason(
        json['target'] as Map<String, dynamic>,
      ),
    );
  }
}

const String mutationCreateQuery = r'''
  BEGIN TRANSACTION;

    LET $created = (CREATE type::table($table) CONTENT $data)[0];
    LET $mutation = CREATE ONLY _spooky_pending_mutations SET
        mutation_type = 'create',
        record_id = $created.id,
        data = $data;

    RETURN {
        target: $created,
        mutation_id: $mutation.id
    };

  COMMIT TRANSACTION;
''';

const String mutationUpdateQuery = r'''
  BEGIN TRANSACTION;

    LET $updated = UPDATE ONLY type::record($id) MERGE $data;
    LET $mutation = CREATE ONLY _spooky_pending_mutations SET
        mutation_type = 'update',
        record_id = type::record($id),
        data = $data;

    RETURN {
        target: $updated,
        mutation_id: $mutation.id
    };

  COMMIT TRANSACTION;
''';

const String mutationDeleteQuery = r'''
  BEGIN TRANSACTION;

    DELETE type::record($id);
    LET $mutation = CREATE ONLY _spooky_pending_mutations SET
        mutation_type = 'delete',
        record_id = type::record($id);
    RETURN {
        mutation_id: $mutation.id
    };

  COMMIT TRANSACTION;
''';
