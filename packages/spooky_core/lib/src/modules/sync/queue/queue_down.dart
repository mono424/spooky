import '../../../events/event_system.dart';
import '../../../services/logger/logger.dart';
import '../sync_events.dart';

/// Download (remote -> local) queue events (TS `DownEvent` union).
sealed class DownEvent {
  String get hash;
}

final class RegisterEvent extends DownEvent {
  RegisterEvent(this.hash);
  @override
  final String hash;
}

final class SyncDownEvent extends DownEvent {
  SyncDownEvent(this.hash);
  @override
  final String hash;
}

final class HeartbeatEvent extends DownEvent {
  HeartbeatEvent(this.hash);
  @override
  final String hash;
}

final class CleanupEvent extends DownEvent {
  CleanupEvent(this.hash);
  @override
  final String hash;
}

/// FIFO queue of [DownEvent]s with enqueue notifications (TS `DownQueue`).
class DownQueue {
  DownQueue(SpookyLogger logger) : _logger = logger.child('DownQueue');

  final List<DownEvent> _queue = [];
  final EventSystem _events = createSyncQueueEventSystem();
  final SpookyLogger _logger;

  EventSystem get events => _events;
  int get size => _queue.length;

  void push(DownEvent event) {
    _queue.add(event);
    _events.addEvent(SpookyEvent(
        SyncQueueEventTypes.queryItemEnqueued, {'queueSize': _queue.length}));
  }

  Future<void> next(Future<void> Function(DownEvent event) fn) async {
    if (_queue.isEmpty) return;
    final event = _queue.removeAt(0);
    try {
      await fn(event);
    } catch (error) {
      _logger.error('Failed to process query', error);
      _queue.insert(0, event);
      rethrow;
    }
  }
}
