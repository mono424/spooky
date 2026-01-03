import 'dart:async';
import '../../database/local.dart';
import '../../mutation/events.dart'; // MutationEvent, MutationPayload
import '../events.dart'; // SyncQueueEventSystem, SyncQueueEventTypes
import '../../events/main.dart'; // BaseEvent, EventSystem
import '../../query/utils.dart'; // extractResult

// Internal type for valid UpEvents used in the queue
abstract class UpEvent {
  final String mutationId;
  final String recordId;
  final Map<String, dynamic>? data;

  UpEvent(this.mutationId, this.recordId, {this.data});
}

class CreateEvent extends UpEvent {
  CreateEvent(String mutationId, String recordId, Map<String, dynamic> data)
    : super(mutationId, recordId, data: data);
}

class UpdateEvent extends UpEvent {
  UpdateEvent(String mutationId, String recordId, Map<String, dynamic> data)
    : super(mutationId, recordId, data: data);
}

class DeleteEvent extends UpEvent {
  DeleteEvent(String mutationId, String recordId) : super(mutationId, recordId);
}

class UpQueue {
  final List<UpEvent> queue = [];
  final EventSystem<SyncQueueEvent> events = EventSystem<SyncQueueEvent>();

  final LocalDatabaseService local;

  UpQueue(this.local);

  int get size => queue.length;

  void push(UpEvent event) {
    queue.add(event);
    events.addEvent(
      MutationEnqueued(MutationEnqueuedPayload(queueSize: queue.length)),
    );
  }

  Future<void> next(Future<void> Function(UpEvent event) fn) async {
    if (queue.isEmpty) return;

    final event = queue.removeAt(0);

    try {
      await fn(event);
    } catch (error) {
      print("Failed to process mutation: $error");
      // Put it back at the front
      queue.insert(0, event);
      rethrow;
    }

    try {
      await removeEventFromDatabase(event.mutationId);
    } catch (error) {
      print("Failed to remove mutation from database: $error");
    }
  }

  Future<void> removeEventFromDatabase(String mutationId) {
    return local.query("DELETE type::record(\$id)", vars: {'id': mutationId});
  }

  void listenForMutations(EventSystem<dynamic> mutationEvents) {
    // Subscribe to MutationEvent
    mutationEvents.subscribe<MutationEvent>((event) {
      for (final mutation in event.payload) {
        _handleMutationPayload(mutation);
      }
    });
  }

  void _handleMutationPayload(MutationPayload mutation) {
    final event = _payloadToUpEvent(mutation);
    if (event != null) push(event);
  }

  void _addToQueueInternal(MutationPayload mutation) {
    final event = _payloadToUpEvent(mutation);
    if (event != null) queue.add(event);
  }

  UpEvent? _payloadToUpEvent(MutationPayload mutation) {
    switch (mutation.type) {
      case MutationAction.create:
        if (mutation.data != null) {
          return CreateEvent(
            mutation.mutation_id,
            mutation.record_id,
            mutation.data!,
          );
        }
        break;
      case MutationAction.update:
        if (mutation.data != null) {
          return UpdateEvent(
            mutation.mutation_id,
            mutation.record_id,
            mutation.data!,
          );
        }
        break;
      case MutationAction.delete:
        return DeleteEvent(mutation.mutation_id, mutation.record_id);
    }
    return null;
  }

  Future<void> loadFromDatabase() async {
    try {
      final resStr = await local.query<String>(
        "SELECT * FROM _spooky_pending_mutations ORDER BY created_at ASC",
      );

      final dynamic raw = extractResult(resStr);
      List<Map<String, dynamic>> mutations = [];

      if (raw is List) {
        mutations = raw.cast<Map<String, dynamic>>();
      }

      // Match TS: Clear queue/cache from DB as authoritative source
      queue.clear();

      for (final r in mutations) {
        final typeStr = r['mutationType'] as String?;
        final recordId = r['recordId'] as String?;
        final data = r['data'] as Map<String, dynamic>?;
        final mutationId = r['id'].toString();

        if (typeStr == null || recordId == null) continue;

        MutationAction? action;
        try {
          action = MutationAction.values.firstWhere((e) => e.name == typeStr);
        } catch (_) {}

        if (action == null) continue;

        final payload = MutationPayload(
          type: action,
          record_id: recordId,
          mutation_id: mutationId,
          data: data,
        );

        // Directly add to queue without emitting events (TS behavior)
        _addToQueueInternal(payload);
      }
    } catch (error) {
      print("Failed to load pending mutations: $error");
    }
  }
}
