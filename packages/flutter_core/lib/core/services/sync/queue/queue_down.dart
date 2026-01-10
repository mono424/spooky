import 'dart:async';
import '../events.dart'; // SyncQueueEventSystem, SyncQueueEventTypes
import '../../events/main.dart'; // BaseEvent, EventSystem
import '../../database/local.dart';
import '../../query/event.dart'; // QueryEvent and types

abstract class DownEvent {
  final String type;
  DownEvent(this.type);
}

// 1. RegisterEvent
class RegisterEvent extends DownEvent {
  final IncantationInitializedPayload payload;
  RegisterEvent(this.payload) : super('register');
}

// 2. SyncEvent
class SyncEvent extends DownEvent {
  final IncantationRemoteHashUpdatePayload payload;
  SyncEvent(this.payload) : super('sync');
}

// 3. HeartbeatEvent
class HeartbeatEvent extends DownEvent {
  final IncantationTTLHeartbeatPayload payload;
  HeartbeatEvent(this.payload) : super('heartbeat');
}

// 4. CleanupEvent
class CleanupEvent extends DownEvent {
  final IncantationCleanupPayload payload;
  CleanupEvent(this.payload) : super('cleanup');
}

class DownQueue {
  final List<DownEvent> queue = [];
  final EventSystem<SyncQueueEvent> events = EventSystem<SyncQueueEvent>();

  final LocalDatabaseService local;

  DownQueue(this.local);

  int get size => queue.length;

  void push(DownEvent event) {
    queue.add(event);
    _emitPushEvent(event);
  }

  void _emitPushEvent(DownEvent event) {
    if (event is RegisterEvent) {
      events.addEvent(
        IncantationRegistrationEnqueued(
          IncantationRegistrationEnqueuedPayload(
            incantationId: event.payload.incantationId,
            surrealql: event.payload.surrealql,
            ttl: event.payload.ttl,
          ),
        ),
      );
    } else if (event is SyncEvent) {
      events.addEvent(
        IncantationSyncEnqueued(
          IncantationSyncEnqueuedPayload(
            incantationId: event.payload.incantationId,
            remoteHash: event.payload.remoteHash,
          ),
        ),
      );
    } else if (event is HeartbeatEvent) {
      events.addEvent(
        IncantationTTLHeartbeatEnqueued(
          IncantationTTLHeartbeatEnqueuedPayload(
            incantationId: event.payload.incantationId,
          ),
        ),
      );
    } else if (event is CleanupEvent) {
      events.addEvent(
        IncantationCleanupEnqueued(
          IncantationCleanupEnqueuedPayload(
            incantationId: event.payload.incantationId,
          ),
        ),
      );
    }
  }

  Future<void> next(Future<void> Function(DownEvent event) fn) async {
    if (queue.isEmpty) return;

    final event = queue.removeAt(0);
    try {
      await fn(event);
    } catch (error) {
      print("Failed to process query event: $error");
      queue.insert(0, event); // Push back
      rethrow;
    }
  }

  void listenForQueries(EventSystem<QueryEvent> queryEvents) {
    queryEvents.subscribe<IncantationInitialized>((event) {
      push(RegisterEvent(event.payload));
    });

    queryEvents.subscribe<IncantationRemoteHashUpdate>((event) {
      push(SyncEvent(event.payload));
    });

    queryEvents.subscribe<IncantationTTLHeartbeat>((event) {
      push(HeartbeatEvent(event.payload));
    });

    queryEvents.subscribe<IncantationCleanup>((event) {
      push(CleanupEvent(event.payload));
    });
  }
}
