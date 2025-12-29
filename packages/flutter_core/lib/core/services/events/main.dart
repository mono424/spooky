import 'dart:async';

/// Die Basisklasse für alle Events.
/// [T] ist der Typ des Payloads (in deinem Fall List<MutationPayload>).
abstract class BaseEvent<T> {
  final String type;
  final T payload;

  BaseEvent(this.type, this.payload);
}

/// Das Event System, generisch auf einen bestimmten Event-Typ [E].
/// In deinem Fall ist E = MutationEvent.
class EventSystem<E extends BaseEvent> {
  // 1. Der Stream Controller
  final StreamController<E> _controller = StreamController<E>.broadcast();

  // 2. Speicher für 'immediately' (letztes Event pro Klasse)
  final Map<Type, E> _lastEvents = {};

  // 3. Buffer für asynchrone Verarbeitung
  final List<E> _buffer = [];
  bool _isProcessing = false;

  EventSystem();

  /// Öffentlicher Stream, falls man direkt zuhören will (optional)
  Stream<E> get stream => _controller.stream;

  /// Fügt ein Event hinzu (entspricht deinem Aufruf im Manager)
  void addEvent(E event) {
    _buffer.add(event);
    _scheduleProcessing();
  }

  /// Verarbeitet den Buffer asynchron (Microtask)
  void _scheduleProcessing() {
    if (_isProcessing) return;

    scheduleMicrotask(() {
      _isProcessing = true;
      while (_buffer.isNotEmpty) {
        final event = _buffer.removeAt(0);

        // Wir nutzen den RuntimeType als Key, das ist sicherer als Strings
        _lastEvents[event.runtimeType] = event;

        _controller.add(event);
      }
      _isProcessing = false;
    });
  }

  /// Abonniert Events eines bestimmten Typs [T].
  /// Da dein EventSystem<MutationEvent> ist, ist T meistens MutationEvent.
  StreamSubscription<T> subscribe<T extends E>(
    void Function(T event) handler, {
    bool immediately = false,
    bool once = false,
  }) {
    return _subscribeInternal<T>(
      T,
      (event) => handler(event as T),
      immediately,
      once,
    );
  }

  /// Helper für die Subscription-Logik
  StreamSubscription<S> _subscribeInternal<S extends E>(
    Type targetType,
    void Function(E event) handler,
    bool immediately,
    bool once,
  ) {
    bool firedImmediately = false;

    // 1. Immediately Logik
    if (immediately) {
      final last = _lastEvents[targetType];
      // Prüfen, ob ein letztes Event existiert
      if (last != null) {
        handler(last);
        firedImmediately = true;
      }
    }

    // Wenn once=true und wir es schon sofort gefeuert haben, sind wir fertig.
    if (once && firedImmediately) {
      return const Stream.empty().listen(null) as StreamSubscription<S>;
    }

    // 2. Stream Filterung
    // Wenn T == E ist (also genau MutationEvent), müssen wir nicht filtern,
    // aber der Einheitlichkeit halber lassen wir den Check drin.
    Stream<E> stream = _controller.stream;

    if (targetType != E) {
      stream = stream.where((e) => e.runtimeType == targetType);
    }

    // 3. Once Logik
    if (once) {
      stream = stream.take(1);
    }

    return stream.cast<S>().listen(handler);
  }

  /// Schließt das System
  void dispose() {
    _controller.close();
    _buffer.clear();
    _lastEvents.clear();
  }
}
