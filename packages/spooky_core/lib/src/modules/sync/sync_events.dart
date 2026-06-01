import '../../events/event_system.dart';

/// Queue-level event type names (TS `SyncQueueEventTypes`).
abstract final class SyncQueueEventTypes {
  static const mutationEnqueued = 'MUTATION_ENQUEUED';
  static const mutationDequeued = 'MUTATION_DEQUEUED';
  static const queryItemEnqueued = 'QUERY_ITEM_ENQUEUED';
}

EventSystem createSyncQueueEventSystem() => EventSystem([
      SyncQueueEventTypes.queryItemEnqueued,
      SyncQueueEventTypes.mutationEnqueued,
      SyncQueueEventTypes.mutationDequeued,
    ]);

/// Sync-level event type names (TS `SyncEventTypes`).
abstract final class SyncEventTypes {
  static const queryUpdated = 'SYNC_QUERY_UPDATED';
  static const remoteDataIngested = 'SYNC_REMOTE_DATA_INGESTED';
  static const mutationRolledBack = 'SYNC_MUTATION_ROLLED_BACK';
}

EventSystem createSyncEventSystem() => EventSystem([
      SyncEventTypes.queryUpdated,
      SyncEventTypes.remoteDataIngested,
      SyncEventTypes.mutationRolledBack,
    ]);
