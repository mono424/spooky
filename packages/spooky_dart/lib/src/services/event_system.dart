import 'package:rxdart/rxdart.dart';

enum SpookyEventType { authChanged, queryChanged, mutation, error }

class SpookyEvent {
  final SpookyEventType type;
  final dynamic payload;

  SpookyEvent(this.type, [this.payload]);
}

class EventSystem {
  final _subject = PublishSubject<SpookyEvent>();

  Stream<SpookyEvent> get onEvent => _subject.stream;

  void emit(SpookyEventType type, [dynamic payload]) {
    _subject.add(SpookyEvent(type, payload));
  }

  Stream<dynamic> on(SpookyEventType type) {
    return _subject.stream.where((e) => e.type == type).map((e) => e.payload);
  }

  void dispose() {
    _subject.close();
  }
}
