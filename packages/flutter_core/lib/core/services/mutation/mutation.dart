import 'dart:convert';
import 'events.dart';
import '../database/main.dart';
import '../events/main.dart';
import './mutation_querys.dart';
import '../database/surreal_decoder.dart';

class MutationManager {
  final LocalDatabaseService db;
  final EventSystem<MutationEvent> events = EventSystem<MutationEvent>();

  MutationManager(this.db);

  EventSystem<MutationEvent> get getEvents => events;

  Future<String> create(String table, Map<String, dynamic> data) async {
    final res = await db.query(
      mutationCreateQuery,
      vars: {'table': table, 'data': data},
    );

    print('schau mal das ist mein res: ');
    print(SurrealDecoder.decode(res, removeNulls: true));

    // Create payload for event/sync
    // Note: In a real app we might parse the result to get the actual ID
    final payload = MutationPayload(
      action: MutationAction.create,
      recordId: '$table:unknown', // Placeholder until we parse result
      mutationId: DateTime.now().toIso8601String(),
      data: data,
    );

    events.addEvent(MutationEvent([payload]));

    return "result";
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
