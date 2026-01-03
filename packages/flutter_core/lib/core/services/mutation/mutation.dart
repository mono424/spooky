import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart'; // RecordId
import 'mutation_querys.dart';
import '../database/main.dart'; // LocalDatabaseService
import 'events.dart';
import '../events/main.dart'; // EventSystem

class MutationManager {
  final LocalDatabaseService db;
  final EventSystem<MutationEvent> events = EventSystem<MutationEvent>();

  MutationManager(this.db);

  EventSystem<MutationEvent> get getEvents => events;

  // Helper for retrying DB operations (matches Core's withRetry)
  Future<T> _withRetry<T>(
    Future<T> Function() operation, {
    int retries = 3,
    int delayMs = 100,
  }) async {
    dynamic lastError;
    for (var i = 0; i < retries; i++) {
      try {
        return await operation();
      } catch (err) {
        lastError = err;
        final msg = err.toString();
        if (msg.contains('Can not open transaction') ||
            msg.contains('transaction') ||
            msg.contains('Database is busy')) {
          print(
            'Retrying DB operation due to transaction error: attempt ${i + 1}',
          );
          await Future.delayed(Duration(milliseconds: delayMs * (i + 1)));
          continue;
        }
        rethrow;
      }
    }
    throw lastError;
  }

  Future<MutationResponse> create(
    RecordId id,
    Map<String, dynamic> data,
  ) async {
    final response = await _withRetry<MutationResponse?>(() async {
      final res = await db.queryTyped<String>(
        mutationCreateQuery,
        vars: {'id': id, 'data': data},
      );

      final [response, ...] =
          SurrealDecoder.decodeNative(res, removeNulls: true) as List;

      if (response != null && response['error'] != null) {
        throw Exception('Mutation Error: ${response['error']}');
      }

      if (response == null) return null;

      return MutationResponse.fromJson(response);
    });

    events.addEvent(
      MutationEvent([
        MutationPayload(
          type: MutationAction.create,
          mutation_id: response!.mutationID,
          record_id: response.target!.id,
          data: data,
        ),
      ]),
    );

    return response;
  }

  Future<MutationResponse> update(
    RecordId id,
    Map<String, dynamic> data,
  ) async {
    final response = await _withRetry(() async {
      final res = await db.queryTyped<String>(
        mutationUpdateQuery,
        vars: {'id': id, 'data': data},
      );

      final [response, ...] =
          SurrealDecoder.decodeNative(res, removeNulls: true) as List;

      if (response != null && response['error'] != null) {
        throw Exception('Mutation Error: ${response['error']}');
      }

      if (response == null) return null;

      return MutationResponse.fromJson(response);
    });

    events.addEvent(
      MutationEvent([
        MutationPayload(
          type: MutationAction.update,
          mutation_id: response!.mutationID,
          record_id: response.target!.id,
          data: data,
        ),
      ]),
    );

    return response;
  }

  Future<void> delete(RecordId id) async {
    final response = await _withRetry(() async {
      final res = await db.queryTyped<String>(
        mutationDeleteQuery,
        vars: {'id': id},
      );

      final [..., response] =
          SurrealDecoder.decodeNative(res, removeNulls: true) as List;

      if (response != null && response['error'] != null) {
        throw Exception('Mutation Error: ${response['error']}');
      }

      if (response == null) return null;

      return MutationResponse.fromJson(response);
    });

    events.addEvent(
      MutationEvent([
        MutationPayload(
          type: MutationAction.delete,
          mutation_id: response!.mutationID,
          record_id: id.toString(),
        ),
      ]),
    );
  }
}
