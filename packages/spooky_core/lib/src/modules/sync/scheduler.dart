import '../../services/logger/logger.dart';
import 'queue/queue_down.dart';
import 'queue/queue_up.dart';
import 'sync_events.dart';

/// Decides when to sync: drains the up-queue, then the down-queue, with the
/// down-queue gated behind a non-empty up-queue (TS `SyncScheduler`).
class SyncScheduler {
  SyncScheduler(
    this._upQueue,
    this._downQueue,
    this._onProcessUp,
    this._onProcessDown,
    SpookyLogger logger, {
    RollbackCallback? onRollback,
  })  : _logger = logger.child('SyncScheduler'),
        _onRollback = onRollback;

  final UpQueue _upQueue;
  final DownQueue _downQueue;
  final Future<void> Function(UpEvent event) _onProcessUp;
  final Future<void> Function(DownEvent event) _onProcessDown;
  // ignore: unused_field
  final SpookyLogger _logger;
  final RollbackCallback? _onRollback;

  bool _isSyncingUp = false;
  bool _isSyncingDown = false;

  Future<void> init() async {
    await _upQueue.loadFromDatabase();
    _upQueue.events
        .subscribe(SyncQueueEventTypes.mutationEnqueued, (_) => syncUp());
    _downQueue.events
        .subscribe(SyncQueueEventTypes.queryItemEnqueued, (_) => syncDown());
  }

  void enqueueMutation(List<UpEvent> mutations) {
    for (final mutation in mutations) {
      _upQueue.push(mutation);
    }
  }

  void enqueueDownEvent(DownEvent event) => _downQueue.push(event);

  Future<void> syncUp() async {
    if (_isSyncingUp) return;
    _isSyncingUp = true;
    try {
      while (_upQueue.size > 0) {
        await _upQueue.next(_onProcessUp, _onRollback);
      }
    } finally {
      _isSyncingUp = false;
      unawaited(syncDown());
    }
  }

  Future<void> syncDown() async {
    if (_isSyncingDown) return;
    if (_upQueue.size > 0) return;
    _isSyncingDown = true;
    try {
      while (_downQueue.size > 0) {
        if (_upQueue.size > 0) break;
        await _downQueue.next(_onProcessDown);
      }
    } finally {
      _isSyncingDown = false;
    }
  }

  bool get isSyncing => _isSyncingUp || _isSyncingDown;
}

void unawaited(Future<void> future) {}
