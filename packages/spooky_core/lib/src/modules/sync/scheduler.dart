import 'dart:async';

import '../../services/logger/logger.dart';
import 'queue/queue_down.dart';
import 'queue/queue_up.dart';
import 'sync_events.dart';

/// Reports the outcome of each drained sync round (TS `onSyncOutcome`).
typedef SyncOutcomeCallback = void Function(bool ok, [Object? error]);

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
    SyncOutcomeCallback? onSyncOutcome,
  })  : _logger = logger.child('SyncScheduler'),
        _onRollback = onRollback,
        _onSyncOutcome = onSyncOutcome;

  final UpQueue _upQueue;
  final DownQueue _downQueue;
  final Future<void> Function(UpEvent event) _onProcessUp;
  final Future<void> Function(DownEvent event) _onProcessDown;
  final SpookyLogger _logger;
  final RollbackCallback? _onRollback;

  /// Reports the outcome of each round that actually processed ≥1 item:
  /// `ok=true` on a clean drain, `ok=false` with the error when the round halted
  /// on a failure. Drives sync-health tracking; empty rounds report nothing.
  final SyncOutcomeCallback? _onSyncOutcome;

  bool _isSyncingUp = false;
  bool _isSyncingDown = false;
  bool _paused = false;
  List<Completer<void>> _pauseWaiters = [];

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

  /// Suspend syncing (TS `pause`). Refuses new rounds and completes once any
  /// in-flight round has finished. The pause point is BETWEEN queue items,
  /// never between an item's remote push and its outbox-row delete, so a
  /// processed mutation's `_00_pending_mutations` delete always lands.
  Future<void> pause() {
    _paused = true;
    if (!_isSyncingUp && !_isSyncingDown) return Future.value();
    final completer = Completer<void>();
    _pauseWaiters.add(completer);
    return completer.future;
  }

  void resume() {
    _paused = false;
    unawaited(syncUp());
  }

  void _maybeResolvePause() {
    if (!_paused || _isSyncingUp || _isSyncingDown) return;
    final waiters = _pauseWaiters;
    _pauseWaiters = [];
    for (final waiter in waiters) {
      if (!waiter.isCompleted) waiter.complete();
    }
  }

  Future<void> syncUp() async {
    if (_isSyncingUp || _paused) return;
    _isSyncingUp = true;
    var processedAny = false;
    try {
      if (_upQueue.size > 0) {
        _logger.debug('Draining up-queue (${_upQueue.size} pending)');
      }
      while (_upQueue.size > 0 && !_paused) {
        await _upQueue.next(_onProcessUp, _onRollback);
        processedAny = true;
      }
      if (processedAny) _onSyncOutcome?.call(true);
    } catch (error) {
      _onSyncOutcome?.call(false, error);
      // syncUp is fire-and-forget: it is wired to the mutationEnqueued event
      // (broadcast synchronously, returned future dropped) and is also kicked
      // off via `unawaited(syncDown())` below. An error escaping here would
      // surface as an unhandled async error with no caller able to catch it.
      // UpQueue.next already logs the failing item and re-queues it for the
      // next trigger, so swallow here to keep the failure contained.
      _logger.debug(
          'syncUp halted on a queue error; item re-queued, will retry on next trigger: $error');
    } finally {
      _isSyncingUp = false;
      _maybeResolvePause();
      unawaited(syncDown());
    }
  }

  Future<void> syncDown() async {
    if (_isSyncingDown || _paused) return;
    if (_upQueue.size > 0) return;
    _isSyncingDown = true;
    var processedAny = false;
    try {
      while (_downQueue.size > 0 && !_paused) {
        if (_upQueue.size > 0) break;
        await _downQueue.next(_onProcessDown);
        processedAny = true;
      }
      if (processedAny) _onSyncOutcome?.call(true);
    } catch (error) {
      _onSyncOutcome?.call(false, error);
      // Same fire-and-forget story as syncUp. The canonical case is a transient
      // remote failure on `fn::query::register`: DownQueue.next logs it and
      // re-queues the event at the head, so stop draining this pass and let the
      // next enqueue retry.
      _logger.debug(
          'syncDown halted on a queue error; item re-queued, will retry on next trigger: $error');
    } finally {
      _isSyncingDown = false;
      _maybeResolvePause();
    }
  }

  bool get isSyncing => _isSyncingUp || _isSyncingDown;

  /// Whether syncing is currently suspended (TS `paused`).
  bool get isPaused => _paused;
}
