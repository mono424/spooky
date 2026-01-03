import '../events/main.dart';

enum MutationAction { create, update, delete }

// Matches core/src/services/mutation/events.ts structure
class MutationPayload {
  final MutationAction type; // 'create', 'update', 'delete'
  final String mutation_id;
  final String record_id;
  final Map<String, dynamic>? data;

  MutationPayload({
    required this.type,
    required this.record_id,
    required this.mutation_id,
    this.data,
  });

  // Helper factory or conversion if needed
}

class MutationEvent extends BaseEvent<List<MutationPayload>> {
  static const String typeName = "MUTATION_CREATED";
  MutationEvent(List<MutationPayload> mutations) : super(typeName, mutations);
}
