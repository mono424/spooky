import 'dart:async';

abstract class BaseEvent<T> {
  final String type;
  final T? payload;
  BaseEvent(this.type, [this.payload]);
}

class EventSystem<T extends BaseEvent> {
  // 1. Der Stream für alle Events
  final _controller = StreamController<T>.broadcast();

  // 2. Speicher für das jeweils letzte Event jedes Typs (für 'immediately')
  final Map<String, T> _lastEvents = {};

  // 3. Buffer für asynchrone Verarbeitung (ähnlich wie im TS Code)
  final List<T> _buffer = [];
  bool _isProcessing = false;

  /// Stream für Abonnenten
  Stream<T> get stream => _controller.stream;

  /// Registriert ein Event und verarbeitet es asynchron
  void addEvent(T event) {
    _buffer.add(event);
    _scheduleProcessing();
  }

  void _scheduleProcessing() {
    if (_isProcessing) return;

    // scheduleMicrotask entspricht queueMicrotask in TS
    scheduleMicrotask(() async {
      _isProcessing = true;
      while (_buffer.isNotEmpty) {
        final event = _buffer.removeAt(0);
        _lastEvents[event.type] = event;
        _controller.add(event);
      }
      _isProcessing = false;
    });
  }

  /// Spezielle Subscribe-Methode für die 'immediately' Funktionalität
  StreamSubscription<T> subscribe(
    void Function(T event) onData, {
    String? filterType,
    bool immediately = false,
    bool once = false,
  }) {
    // Falls sofortiges Event gewünscht:
    if (immediately &&
        filterType != null &&
        _lastEvents.containsKey(filterType)) {
      onData(_lastEvents[filterType]!);
    }

    Stream<T> currentStream = stream;

    // Filtern nach Typ falls gewünscht
    if (filterType != null) {
      currentStream = currentStream.where((e) => e.type == filterType);
    }

    if (once) {
      // Dart hat 'firstWhere' oder 'take(1)' für 'once' Funktionalität
      final subscription = currentStream.take(1).listen(onData);
      return subscription;
    }

    return currentStream.listen(onData);
  }

  void dispose() {
    _controller.close();
  }
}
