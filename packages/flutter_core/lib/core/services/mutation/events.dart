import '../events/main.dart';

enum MutationAction { create, update, delete }

class MutationPayload {
  final MutationAction action;
  final String mutationId;
  final String recordId;
  final Map<String, dynamic>? data;

  MutationPayload({
    required this.action,
    required this.recordId,
    required this.mutationId,
    this.data,
  });
}

// Konkrete Implementierung deines Events
class MutationEvent extends BaseEvent<List<MutationPayload>> {
  static const String typeName = "MUTATION_CREATED";
  MutationEvent(List<MutationPayload> mutations) : super(typeName, mutations);
}
