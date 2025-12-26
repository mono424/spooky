import 'dart:convert';
import 'events.dart';
import '../database/main.dart';
import '../events/main.dart';
import './mutation_querys.dart';
import '../database/surreal_decoder.dart';
import '../../types.dart';

class MutationManager {
  final LocalDatabaseService db;
  final EventSystem<MutationEvent> events = EventSystem<MutationEvent>();

  MutationManager(this.db);

  EventSystem<MutationEvent> get getEvents => events;

  Future<Map<String, dynamic>?> create(
    String table,
    Map<String, dynamic> data,
  ) async {
    final resRaw = await db.query(
      mutationCreateQuery,
      vars: {'table': table, 'data': data},
    );

    final [response, ...] =
        SurrealDecoder.decodeNative(resRaw, removeNulls: true) as List;

    if (response != null && response['error'] != null) {
      throw Exception('Mutation Error: ${response['error']}');
    }

    if (response == null) return null;

    final mutationResponse = MutationResponse.fromJson(response);
    // Create payload for event/sync
    // Note: In a real app we might parse the result to get the actual ID
    final payload = MutationPayload(
      action: MutationAction.create,
      recordId: mutationResponse.target.id, // Placeholder until we parse result
      mutationId: mutationResponse.mutationID,
      data: data,
    );

    events.addEvent(MutationEvent([payload]));

    return mutationResponse.target.record;
  }

  Future<Map<String, dynamic>?> update(
    String rid,
    Map<String, dynamic> data,
  ) async {
    final resRaw = await db.query(
      mutationUpdateQuery,
      vars: {'id': rid, 'data': data},
    );

    final [response, ...] =
        SurrealDecoder.decodeNative(resRaw, removeNulls: true) as List;

    if (response != null && response['error'] != null) {
      throw Exception('Mutation Error: ${response['error']}');
    }

    if (response == null) return null;

    final mutationResponse = MutationResponse.fromJson(response);

    final payload = MutationPayload(
      action: MutationAction.update,
      recordId: mutationResponse.target.id,
      mutationId: mutationResponse.mutationID,
      data: data,
    );
    events.addEvent(MutationEvent([payload]));

    return mutationResponse.target.record;
  }

  Future<void> delete(String rid) async {
    final resRaw = await db.query(mutationDeleteQuery, vars: {'id': rid});
    final [..., response] =
        SurrealDecoder.decodeNative(resRaw, removeNulls: true) as List;

    final payload = MutationPayload(
      action: MutationAction.delete,
      recordId: rid,
      mutationId: response['mutation_id'],
    );
    events.addEvent(MutationEvent([payload]));
  }
}
