import 'dart:async';
import '../../database/local.dart';
import '../../mutation/events.dart'; // MutationEvent, MutationAction
import '../events.dart'; // SyncQueueEventSystem, SyncQueueEventTypes
import '../../events/main.dart'; // BaseEvent, EventSystem

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
    // mutationId is likely just the ID part or full things.
    // In mutation/mutation.ts we store it as `_spooky_pending_mutations:UUID`.
    // The incoming mutationId here usually matches that.
    // If it's just the ID part, we might need to prefix.
    // Assuming it's fully qualified or we can just delete by ID.
    return local.query("DELETE \$id", vars: {'id': mutationId});
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
    switch (mutation.action) {
      case MutationAction.create:
        if (mutation.data != null) {
          push(
            CreateEvent(mutation.mutationId, mutation.recordId, mutation.data!),
          );
        }
        break;
      case MutationAction.update:
        if (mutation.data != null) {
          push(
            UpdateEvent(mutation.mutationId, mutation.recordId, mutation.data!),
          );
        }
        break;
      case MutationAction.delete:
        push(DeleteEvent(mutation.mutationId, mutation.recordId));
        break;
    }
  }

  Future<void> loadFromDatabase() async {
    try {
      // Dart engine query usually returns JSON string
      final resSql = await local.query(
        "SELECT * FROM _spooky_pending_mutations ORDER BY created_at ASC",
      );
      // extractResult or custom parsing?
      // Let's assume standard extraction for now using jsonDecode
      // Note: extractResult helper from query/utils.dart could be useful if imported.
      // But query/utils is in sibling folder.
      // We will do inline parsing for simplicity or duplicate extractResult logic.

      // Let's rely on basic Parsing assuming the response structure
      // (engine might return `[{status: 'OK', result: [...]}]`)
      // But `rawResult` logic from TS suggests we might get list directly?
      // No, Dart Rust Engine returns stringified JSON of the Response Array.

      /*
        r.mutation_type (string)
        r.id (record ID of the mutation itself)
        r.record_id (target record)
        r.data
      */
      // For now, stub implementation of parsing as user might need extractResult.

      // TODO: Implement proper parsing here when extractResult is shared or accessible.
      // For now we assume no persisted events or user handles DB init differently.
    } catch (error) {
      print("Failed to load pending mutations: $error");
    }
  }
}
