import 'dart:async';

/// A single event: a string [type] and an arbitrary [payload].
///
/// The TS system is generic over an `EventTypeMap`; Dart keeps payloads
/// dynamic at the boundary and relies on typed facades / call sites for
/// safety. Mirrors the TS `Event<E, T>` shape (`{ type, payload }`).
class SpookyEvent {
  const SpookyEvent(this.type, this.payload);
  final String type;
  final dynamic payload;
}

typedef SpookyEventHandler = void Function(SpookyEvent event);

/// Debounce configuration for [EventSystem.addEvent] (TS `PushEventOptions`).
class PushEventOptions {
  const PushEventOptions({this.debounced});
  final DebouncedConfig? debounced;
}

class DebouncedConfig {
  const DebouncedConfig({required this.key, required this.delay});
  final String key;
  final int delay;
}

/// Subscription options (TS `EventSubscriptionOptions`).
class EventSubscriptionOptions {
  const EventSubscriptionOptions({this.immediately = false, this.once = false});
  final bool immediately;
  final bool once;
}

class _InnerHandler {
  _InnerHandler(this.id, this.handler, this.once);
  final int id;
  final SpookyEventHandler handler;
  final bool once;
}

/// Type-safe pub/sub with microtask-batched delivery, last-event replay, and
/// key/delay debounce. Faithful port of the TS `EventSystem`.
///
/// Kept as a callback registry (not Dart `Stream`s) to preserve numeric
/// subscription ids, the buffering/`lastEvents` semantics, and `once` /
/// `immediately` behavior the modules depend on.
class EventSystem {
  EventSystem(this._eventTypes) {
    for (final t in _eventTypes) {
      _subscribers[t] = <int, _InnerHandler>{};
    }
  }

  final List<String> _eventTypes;
  int _subscriberId = 0;
  bool _isProcessing = false;
  final List<SpookyEvent> _buffer = [];
  final Map<String, Map<int, _InnerHandler>> _subscribers = {};
  final Map<int, String> _subscribersTypeMap = {};
  final Map<String, SpookyEvent> _lastEvents = {};
  final Map<String, Timer> _debouncedEvents = {};

  List<String> get eventTypes => _eventTypes;

  /// Subscribe [handler] to [type]; returns an id for [unsubscribe].
  int subscribe(
    String type,
    SpookyEventHandler handler, [
    EventSubscriptionOptions options = const EventSubscriptionOptions(),
  ]) {
    final id = _subscriberId++;
    _subscribers[type]![id] = _InnerHandler(id, handler, options.once);
    _subscribersTypeMap[id] = type;
    if (options.immediately) {
      final last = _lastEvents[type];
      if (last != null) handler(last);
    }
    return id;
  }

  /// Subscribe one handler to several event types; returns all ids.
  List<int> subscribeMany(
    List<String> types,
    SpookyEventHandler handler, [
    EventSubscriptionOptions options = const EventSubscriptionOptions(),
  ]) =>
      types.map((t) => subscribe(t, handler, options)).toList();

  bool unsubscribe(int id) {
    final type = _subscribersTypeMap[id];
    if (type != null) {
      _subscribers[type]!.remove(id);
      _subscribersTypeMap.remove(id);
      return true;
    }
    return false;
  }

  /// Emit [type] with [payload].
  void emit(String type, dynamic payload) =>
      addEvent(SpookyEvent(type, payload));

  /// Add a fully-constructed event, optionally debounced by key.
  void addEvent(SpookyEvent event, [PushEventOptions? options]) {
    final debounced = options?.debounced;
    if (debounced != null) {
      _handleDebouncedEvent(event, debounced.key, debounced.delay);
      return;
    }
    _buffer.add(event);
    _scheduleProcessing();
  }

  void _handleDebouncedEvent(SpookyEvent event, String key, int delay) {
    _debouncedEvents[key]?.cancel();
    _debouncedEvents[key] = Timer(Duration(milliseconds: delay), () {
      _debouncedEvents.remove(key);
      _buffer.add(event);
      _scheduleProcessing();
    });
  }

  void _scheduleProcessing() {
    if (!_isProcessing) {
      scheduleMicrotask(_processEvents);
    }
  }

  void _processEvents() {
    if (_isProcessing) return;
    _isProcessing = true;
    try {
      while (_dequeue()) {}
    } finally {
      _isProcessing = false;
    }
  }

  bool _dequeue() {
    if (_buffer.isEmpty) return false;
    final event = _buffer.removeAt(0);
    _lastEvents[event.type] = event;
    _broadcastEvent(event.type, event);
    return true;
  }

  void _broadcastEvent(String type, SpookyEvent event) {
    // Copy to a list so `once` unsubscribes don't mutate during iteration.
    for (final subscriber in _subscribers[type]!.values.toList()) {
      subscriber.handler(event);
      if (subscriber.once) {
        unsubscribe(subscriber.id);
      }
    }
  }
}

EventSystem createEventSystem(List<String> eventTypes) =>
    EventSystem(eventTypes);
