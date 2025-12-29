import '../../types.dart';
import '../events/main.dart';

// Event Types constants
class SyncQueueEventTypes {
  static const String MutationEnqueued = "MUTATION_ENQUEUED";
  static const String IncantationRegistrationEnqueued =
      "INCANTATION_REGISTRATION_ENQUEUED";
  static const String IncantationSyncEnqueued = "INCANTATION_SYNC_ENQUEUED";
  static const String IncantationTTLHeartbeatEnqueued =
      "INCANTATION_TTL_HEARTBEAT_ENQUEUED";
  static const String IncantationCleanupEnqueued =
      "INCANTATION_CLEANUP_ENQUEUED";
}

// Base class for Sync Events
abstract class SyncQueueEvent extends BaseEvent<dynamic> {
  SyncQueueEvent(String type, dynamic payload) : super(type, payload);
}

// --- 1. MutationEnqueued ---

class MutationEnqueuedPayload {
  final int queueSize;
  MutationEnqueuedPayload({required this.queueSize});
}

class MutationEnqueued extends SyncQueueEvent {
  @override
  final MutationEnqueuedPayload payload;
  MutationEnqueued(this.payload)
    : super(SyncQueueEventTypes.MutationEnqueued, payload);
}

// --- 2. IncantationRegistrationEnqueued ---

class IncantationRegistrationEnqueuedPayload {
  final String incantationId;
  final String surrealql;
  final QueryTimeToLive ttl;

  IncantationRegistrationEnqueuedPayload({
    required this.incantationId,
    required this.surrealql,
    required this.ttl,
  });
}

class IncantationRegistrationEnqueued extends SyncQueueEvent {
  @override
  final IncantationRegistrationEnqueuedPayload payload;
  IncantationRegistrationEnqueued(this.payload)
    : super(SyncQueueEventTypes.IncantationRegistrationEnqueued, payload);
}

// --- 3. IncantationSyncEnqueued ---

class IncantationSyncEnqueuedPayload {
  final String incantationId;
  final String remoteHash;

  IncantationSyncEnqueuedPayload({
    required this.incantationId,
    required this.remoteHash,
  });
}

class IncantationSyncEnqueued extends SyncQueueEvent {
  @override
  final IncantationSyncEnqueuedPayload payload;
  IncantationSyncEnqueued(this.payload)
    : super(SyncQueueEventTypes.IncantationSyncEnqueued, payload);
}

// --- 4. IncantationTTLHeartbeatEnqueued ---

class IncantationTTLHeartbeatEnqueuedPayload {
  final String incantationId;
  IncantationTTLHeartbeatEnqueuedPayload({required this.incantationId});
}

class IncantationTTLHeartbeatEnqueued extends SyncQueueEvent {
  @override
  final IncantationTTLHeartbeatEnqueuedPayload payload;
  IncantationTTLHeartbeatEnqueued(this.payload)
    : super(SyncQueueEventTypes.IncantationTTLHeartbeatEnqueued, payload);
}

// --- 5. IncantationCleanupEnqueued ---

class IncantationCleanupEnqueuedPayload {
  final String incantationId;
  IncantationCleanupEnqueuedPayload({required this.incantationId});
}

class IncantationCleanupEnqueued extends SyncQueueEvent {
  @override
  final IncantationCleanupEnqueuedPayload payload;
  IncantationCleanupEnqueued(this.payload)
    : super(SyncQueueEventTypes.IncantationCleanupEnqueued, payload);
}
