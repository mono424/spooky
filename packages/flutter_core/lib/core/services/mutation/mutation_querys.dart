class MutationTargetResponse {
  final String id;
  final Map<String, dynamic> record;

  MutationTargetResponse({required this.id, required this.record});

  factory MutationTargetResponse.fromJson(Map<String, dynamic> json) {
    return MutationTargetResponse(id: json['id'] as String, record: json);
  }
}

class MutationResponse {
  final String mutationID;
  final MutationTargetResponse? target;

  MutationResponse({required this.mutationID, this.target});

  factory MutationResponse.fromJson(Map<String, dynamic> json) {
    return MutationResponse(
      mutationID: json['mutation_id'] as String,
      target: json['target'] != null
          ? MutationTargetResponse.fromJson(
              json['target'] as Map<String, dynamic>,
            )
          : null,
    );
  }
}

const String mutationCreateQuery = r'''
  BEGIN TRANSACTION;

    LET $created = CREATE ONLY type::record($id) CONTENT $data;
    LET $mutation = CREATE ONLY _spooky_pending_mutations CONTENT {
        mutationType: 'create',
        recordId: $created.id,
        data: $data
    };

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
        mutationType = 'update',
        recordId = type::record($id),
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
        mutationType = 'delete',
        recordId = type::record($id);
    RETURN {
        mutation_id: $mutation.id
    };

  COMMIT TRANSACTION;
''';
