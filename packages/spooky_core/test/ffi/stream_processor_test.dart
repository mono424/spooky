import 'package:spooky_core/src/ffi/stream_processor.dart';
import 'package:spooky_core/src/ffi/stream_update.dart';
import 'package:test/test.dart';

void main() {
  group('StreamProcessor (FFI round-trip)', () {
    late StreamProcessor sp;

    setUp(() => sp = StreamProcessor.create());
    tearDown(() => sp.dispose());

    test('registers a view and ingests a matching record', () {
      sp.setPermission('thread', 'true');
      final initial = sp.registerView({
        'id': 'q1',
        'surql': 'SELECT * FROM thread',
        'params': <String, dynamic>{},
        'clientId': 'local',
        'ttl': '10m',
        'lastActiveAt': '2026-01-01T00:00:00.000Z',
      });
      expect(initial, isNotNull);
      expect(initial!.queryHash, 'q1');
      expect(initial.localArray, isEmpty);

      final updates = sp.ingest('thread', 'CREATE', 'thread:a', {
        'id': 'thread:a',
        'title': 'hello',
      });

      expect(updates, isNotEmpty);
      final u = updates.firstWhere((e) => e.queryHash == 'q1');
      expect(u.localArray.map((e) => e.$1), contains('thread:a'));
      expect(u.op, 'CREATE');
      expect(u.materializationTimeMs,
          isNull); // set by the service, not the processor
    });

    test('save/load state round-trips', () {
      sp.setPermission('thread', 'true');
      sp.registerView({
        'id': 'q2',
        'surql': 'SELECT * FROM thread',
        'params': <String, dynamic>{},
        'clientId': 'local',
        'ttl': '10m',
        'lastActiveAt': '2026-01-01T00:00:00.000Z',
      });
      sp.ingest(
          'thread', 'CREATE', 'thread:b', {'id': 'thread:b', 'title': 'x'});

      final state = sp.saveState();
      expect(state, isNotEmpty);

      final sp2 = StreamProcessor.create();
      addTearDown(sp2.dispose);
      sp2.loadState(state); // must not throw
    });

    test('surfaces native errors as SspException', () {
      expect(
        () => sp.registerView({'surql': 'SELECT * FROM thread'}), // missing id
        throwsA(isA<SspException>()),
      );
    });
  });
}
