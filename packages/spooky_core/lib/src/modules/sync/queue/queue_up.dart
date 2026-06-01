import 'dart:async';

import '../../../events/event_system.dart';
import '../../../services/database/local_database_service.dart';
import '../../../services/logger/logger.dart';
import '../../../surreal/value.dart';
import '../../../utils/error_classification.dart';
import '../../../utils/record_id_utils.dart';
import '../sync_events.dart';

/// Outgoing mutation events (TS `UpEvent` discriminated union).
sealed class UpEvent {
  RecordId get mutationId;
  RecordId get recordId;
  PushEventOptions? get options;
}

/// A record creation (TS `CreateEvent`).
final class CreateEvent extends UpEvent {
  CreateEvent({
    required this.mutationId,
    required this.recordId,
    required this.data,
    this.record,
    this.tableName,
    this.options,
  });

  @override
  final RecordId mutationId;
  @override
  final RecordId recordId;
  final Map<String, dynamic> data;
  final Map<String, dynamic>? record;
  final String? tableName;
  @override
  final PushEventOptions? options;
}

/// A record update (TS `UpdateEvent`). [beforeRecord] is mutable because the
/// up-queue patches it when coalescing debounced mutations.
final class UpdateEvent extends UpEvent {
  UpdateEvent({
    required this.mutationId,
    required this.recordId,
    required this.data,
    this.record,
    this.beforeRecord,
    this.options,
  });

  @override
  final RecordId mutationId;
  @override
  final RecordId recordId;
  final Map<String, dynamic> data;
  final Map<String, dynamic>? record;
  Map<String, dynamic>? beforeRecord;
  @override
  final PushEventOptions? options;
}

/// A record deletion (TS `DeleteEvent`).
final class DeleteEvent extends UpEvent {
  DeleteEvent({
    required this.mutationId,
    required this.recordId,
    this.options,
  });

  @override
  final RecordId mutationId;
  @override
  final RecordId recordId;
  @override
  final PushEventOptions? options;
}

/// Notified with a batch of mutations to enqueue for sync (TS `MutationCallback`).
typedef MutationCallback = void Function(List<UpEvent> mutations);

/// Called when a mutation fails with a terminal (application) error so the
/// caller can roll back optimistic local state (TS `RollbackCallback`).
typedef RollbackCallback = Future<void> Function(UpEvent event, Object error);

class _Debounced {
  _Debounced(this.timer, this.firstBeforeRecord);
  Timer timer;
  Map<String, dynamic>? firstBeforeRecord;
}

/// Outgoing mutation queue with debounce coalescing and durable backing in the
/// `_00_pending_mutations` outbox (TS `UpQueue`). Adapted to the sqlite
/// [LocalDatabaseService] document API.
class UpQueue {
  UpQueue(this._local, SpookyLogger logger) : _logger = logger.child('UpQueue');

  final LocalDatabaseService _local;
  final SpookyLogger _logger;
  final EventSystem _events = createSyncQueueEventSystem();
  List<UpEvent> _queue = [];
  final Map<String, _Debounced> _debounced = {};

  EventSystem get events => _events;
  int get size => _queue.length;

  void push(UpEvent event) {
    final debounced = event.options?.debounced;
    if (debounced != null) {
      _handleDebounced(event, debounced.key, debounced.delay);
      return;
    }
    _addToQueue(event);
  }

  void _addToQueue(UpEvent event) {
    _queue.add(event);
    _events.addEvent(SpookyEvent(
        SyncQueueEventTypes.mutationEnqueued, {'queueSize': _queue.length}));
  }

  void _handleDebounced(UpEvent event, String key, int delay) {
    final existing = _debounced[key];
    Map<String, dynamic>? firstBeforeRecord;
    if (existing != null) {
      existing.timer.cancel();
      firstBeforeRecord = existing.firstBeforeRecord;
    } else if (event is UpdateEvent) {
      firstBeforeRecord = event.beforeRecord;
    }

    final timer = Timer(Duration(milliseconds: delay), () {
      _debounced.remove(key);
      if (firstBeforeRecord != null && event is UpdateEvent) {
        event.beforeRecord = firstBeforeRecord;
      }
      _addToQueue(event);
    });
    _debounced[key] = _Debounced(timer, firstBeforeRecord);
  }

  Future<void> next(
    Future<void> Function(UpEvent event) fn, [
    RollbackCallback? onRollback,
  ]) async {
    if (_queue.isEmpty) return;
    final event = _queue.removeAt(0);
    try {
      await fn(event);
    } catch (error) {
      final type = classifySyncError(error);
      if (type == 'network') {
        _logger.error('Network error processing mutation, re-queuing', error);
        _queue.insert(0, event);
        rethrow;
      }
      // Application error -> roll back instead of re-queuing.
      _logger.error(
          'Application error processing mutation, rolling back', error);
      _removeFromDatabase(event.mutationId);
      if (onRollback != null) {
        try {
          await onRollback(event, error);
        } catch (rollbackError) {
          _logger.error('Rollback handler failed', rollbackError);
        }
      }
      _events.addEvent(SpookyEvent(
          SyncQueueEventTypes.mutationDequeued, {'queueSize': _queue.length}));
      return;
    }
    _removeFromDatabase(event.mutationId);
    _events.addEvent(SpookyEvent(
        SyncQueueEventTypes.mutationDequeued, {'queueSize': _queue.length}));
  }

  void _removeFromDatabase(RecordId mutationId) {
    _local.deleteMutation(encodeRecordId(mutationId));
  }

  /// Rebuild the queue from the durable outbox on startup (TS `loadFromDatabase`).
  Future<void> loadFromDatabase() async {
    try {
      final records = _local.getAllMutations();
      _queue = records
          .map<UpEvent?>((r) {
            final id = r['id'] as String? ?? '';
            final recordId = r['recordId'] as String? ?? '';
            switch (r['mutationType']) {
              case 'create':
                return CreateEvent(
                  mutationId: parseRecordIdString(id),
                  recordId: parseRecordIdString(recordId),
                  data: (r['data'] as Map?)?.cast<String, dynamic>() ?? {},
                  tableName: extractTablePart(recordId),
                );
              case 'update':
                return UpdateEvent(
                  mutationId: parseRecordIdString(id),
                  recordId: parseRecordIdString(recordId),
                  data: (r['data'] as Map?)?.cast<String, dynamic>() ?? {},
                  beforeRecord:
                      (r['beforeRecord'] as Map?)?.cast<String, dynamic>(),
                );
              case 'delete':
                return DeleteEvent(
                  mutationId: parseRecordIdString(id),
                  recordId: parseRecordIdString(recordId),
                );
              default:
                _logger.warn('Unknown mutation type ${r['mutationType']}');
                return null;
            }
          })
          .whereType<UpEvent>()
          .toList();
    } catch (error) {
      _logger.error('Failed to load pending mutations from database', error);
    }
  }
}
