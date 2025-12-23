import 'events.dart';
import '../database/main.dart';

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

// --- DIE ANWENDUNG ---

class MutationManager {
  final LocalDatabaseService db;
  final EventSystem<MutationEvent> events = EventSystem<MutationEvent>();

  MutationManager(this.db);

  EventSystem<MutationEvent> get getEvents => events;

  void create(String table, Map<String, dynamic> data) async {
    final payload = MutationPayload(
      action: MutationAction.create,
      recordId: 'user:gc1ka1418u5lr3dbj6ht',
      mutationId: '_spooky_pending_mutations:vuhdhayomnbcx6ny5cn8',
      data: data,
    );

    // Hinzufügen über deine addEvent Methode
    events.addEvent(MutationEvent([payload]));
  }
}
