import 'dart:convert';
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

  Future<String> create(String table, Map<String, dynamic> data) async {
    final json = jsonEncode(data);
    final result = await db.getClient.create(resource: table, data: json);

    // Create payload for event/sync
    // Note: In a real app we might parse the result to get the actual ID
    final payload = MutationPayload(
      action: MutationAction.create,
      recordId: '$table:unknown', // Placeholder until we parse result
      mutationId: DateTime.now().toIso8601String(),
      data: data,
    );
    events.addEvent(MutationEvent([payload]));

    return result;
  }

  Future<String> update(
    String table,
    String id,
    Map<String, dynamic> data,
  ) async {
    final json = jsonEncode(data);
    final result = await db.getClient.update(
      resource: '$table:$id',
      data: json,
    );

    final payload = MutationPayload(
      action: MutationAction.update,
      recordId: '$table:$id',
      mutationId: DateTime.now().toIso8601String(),
      data: data,
    );
    events.addEvent(MutationEvent([payload]));

    return result;
  }

  Future<String> delete(String table, String id) async {
    final result = await db.getClient.delete(resource: '$table:$id');

    final payload = MutationPayload(
      action: MutationAction.delete,
      recordId: '$table:$id',
      mutationId: DateTime.now().toIso8601String(),
    );
    events.addEvent(MutationEvent([payload]));

    return result;
  }
}
