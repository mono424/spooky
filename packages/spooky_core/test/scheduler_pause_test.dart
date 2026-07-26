import 'dart:async';

import 'package:spooky_core/spooky_core.dart';
import 'package:spooky_core/src/modules/sync/queue/queue_down.dart';
import 'package:spooky_core/src/modules/sync/queue/queue_up.dart';
import 'package:spooky_core/src/modules/sync/scheduler.dart';
import 'package:spooky_core/src/services/database/local_database_service.dart';
import 'package:spooky_core/src/services/logger/logger.dart';
import 'package:test/test.dart';

/// `SyncScheduler.pause()` must (a) let an IN-FLIGHT queue item finish —
/// including its outbox-row delete — and only then complete, and (b) refuse new
/// rounds until `resume()`. The pause point is between items. Also covers
/// `onSyncOutcome`, which feeds the consumer's sync-health tracking.
void main() {
  final logger = SpookyLogger.root('test');

  CreateEvent create(String id) => CreateEvent(
        mutationId: RecordId('_00_pending_mutations', id),
        recordId: RecordId.parse('thread:$id'),
        data: {'title': id},
        tableName: 'thread',
      );

  late LocalDatabaseService db;
  setUp(() => db = LocalDatabaseService.open(logger)..provision());
  tearDown(() => db.close());

  group('pause / resume', () {
    test('waits for the in-flight item before completing', () async {
      final up = UpQueue(db, logger)
        ..push(create('a'))
        ..push(create('b'));
      final gate = Completer<void>();
      final processed = <String>[];
      final scheduler = SyncScheduler(
        up,
        DownQueue(logger),
        (e) async {
          await gate.future; // simulate a slow remote push
          processed.add(e.recordId.encode());
        },
        (_) async {},
        logger,
      );

      final round = scheduler.syncUp();
      var pauseDone = false;
      final pause = scheduler.pause().then((_) => pauseDone = true);

      // The in-flight item hasn't finished, so pause must not have completed.
      await Future<void>.delayed(Duration.zero);
      expect(pauseDone, isFalse);

      gate.complete();
      await pause;
      await round;

      // Exactly the in-flight item completed; the second stayed queued.
      expect(processed, ['thread:a']);
      expect(up.size, 1);
    });

    test('refuses new rounds while paused and drains them on resume', () async {
      final up = UpQueue(db, logger)..push(create('a'));
      final processed = <String>[];
      final scheduler = SyncScheduler(
        up,
        DownQueue(logger),
        (e) async => processed.add(e.recordId.encode()),
        (_) async {},
        logger,
      );

      await scheduler.pause();
      expect(scheduler.isPaused, isTrue);
      await scheduler.syncUp();
      expect(processed, isEmpty);

      scheduler.resume();
      await Future<void>.delayed(const Duration(milliseconds: 20));
      expect(processed, ['thread:a']);
      expect(up.size, 0);
    });

    test('completes immediately when nothing is in flight', () async {
      final scheduler = SyncScheduler(UpQueue(db, logger), DownQueue(logger),
          (_) async {}, (_) async {}, logger);
      await expectLater(scheduler.pause(), completes);
    });

    test('a paused down-queue drain is refused too', () async {
      final down = DownQueue(logger)..push(RegisterEvent('h1'));
      final scheduler = SyncScheduler(UpQueue(db, logger), down, (_) async {},
          (_) async => throw StateError('should not run'), logger);
      await scheduler.pause();
      await scheduler.syncDown();
      expect(down.size, 1);
    });
  });

  group('onSyncOutcome', () {
    test('reports ok after a round that processed an item', () async {
      final up = UpQueue(db, logger)..push(create('a'));
      final outcomes = <(bool, Object?)>[];
      final scheduler = SyncScheduler(
        up,
        DownQueue(logger),
        (_) async {},
        (_) async {},
        logger,
        onSyncOutcome: (ok, [error]) => outcomes.add((ok, error)),
      );

      await scheduler.syncUp();
      expect(outcomes.map((o) => o.$1), [true]);
    });

    test('reports the error when a round halts, and swallows it', () async {
      final up = UpQueue(db, logger)..push(create('a'));
      final outcomes = <(bool, Object?)>[];
      final scheduler = SyncScheduler(
        up,
        DownQueue(logger),
        // Network-classified: UpQueue re-queues the item and rethrows.
        (_) async => throw Exception('connection refused'),
        (_) async {},
        logger,
        onSyncOutcome: (ok, [error]) => outcomes.add((ok, error)),
      );

      // The halt must not escape as an unhandled async error.
      await expectLater(scheduler.syncUp(), completes);
      expect(outcomes, hasLength(1));
      expect(outcomes.single.$1, isFalse);
      expect(outcomes.single.$2.toString(), contains('connection refused'));
      expect(up.size, 1); // re-queued for the next trigger
    });

    test('reports nothing for an empty round', () async {
      final outcomes = <bool>[];
      final scheduler = SyncScheduler(
        UpQueue(db, logger),
        DownQueue(logger),
        (_) async {},
        (_) async {},
        logger,
        onSyncOutcome: (ok, [error]) => outcomes.add(ok),
      );

      await scheduler.syncUp();
      await scheduler.syncDown();
      expect(outcomes, isEmpty);
    });

    test('reports ok after a drained down-queue round', () async {
      final down = DownQueue(logger)..push(RegisterEvent('h1'));
      final outcomes = <bool>[];
      final scheduler = SyncScheduler(
        UpQueue(db, logger),
        down,
        (_) async {},
        (_) async {},
        logger,
        onSyncOutcome: (ok, [error]) => outcomes.add(ok),
      );

      await scheduler.syncDown();
      expect(outcomes, [true]);
    });
  });
}
