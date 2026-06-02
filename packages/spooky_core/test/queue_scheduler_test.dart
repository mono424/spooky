import 'dart:async';

import 'package:fake_async/fake_async.dart';
import 'package:spooky_core/spooky_core.dart';
import 'package:spooky_core/src/events/event_system.dart';
import 'package:spooky_core/src/modules/sync/queue/queue_down.dart';
import 'package:spooky_core/src/modules/sync/queue/queue_up.dart';
import 'package:spooky_core/src/modules/sync/scheduler.dart';
import 'package:spooky_core/src/modules/sync/sync_events.dart';
import 'package:spooky_core/src/services/database/local_database_service.dart';
import 'package:spooky_core/src/services/logger/logger.dart';
import 'package:test/test.dart';

void main() {
  final logger = SpookyLogger.root('test');

  CreateEvent create(String id) => CreateEvent(
        mutationId: RecordId('_00_pending_mutations', id),
        recordId: RecordId.parse('thread:$id'),
        data: {'title': id},
        tableName: 'thread',
      );

  group('UpQueue', () {
    late LocalDatabaseService db;
    late UpQueue q;
    setUp(() {
      db = LocalDatabaseService.open(logger)..provision();
      q = UpQueue(db, logger);
    });
    tearDown(() => db.close());

    test('push enqueues and emits MutationEnqueued', () async {
      var size = -1;
      q.events.subscribe(SyncQueueEventTypes.mutationEnqueued,
          (e) => size = (e.payload as Map)['queueSize'] as int);
      q.push(create('a'));
      await Future<void>.value();
      expect(q.size, 1);
      expect(size, 1);
    });

    test('next() processes and dequeues on success', () async {
      q.push(create('a'));
      final processed = <String>[];
      await q.next((e) async => processed.add(e.recordId.encode()));
      expect(processed, ['thread:a']);
      expect(q.size, 0);
    });

    test('network error re-queues and rethrows', () async {
      q.push(create('a'));
      await expectLater(
        q.next((e) async => throw Exception('connection refused')),
        throwsA(isA<Exception>()),
      );
      expect(q.size, 1); // re-queued
    });

    test('application error rolls back (no re-queue) and calls onRollback',
        () async {
      q.push(create('a'));
      UpEvent? rolledBack;
      await q.next(
        (e) async => throw StateError('bad data'),
        (event, err) async => rolledBack = event,
      );
      expect(q.size, 0); // dropped
      expect(rolledBack, isNotNull);
    });

    test('debounced pushes coalesce to one, preserving first beforeRecord', () {
      fakeAsync((async) {
        final u1 = UpdateEvent(
          mutationId: RecordId('_00_pending_mutations', 'm1'),
          recordId: RecordId.parse('thread:x'),
          data: {'title': 'a'},
          beforeRecord: {'title': 'original'},
          options: const PushEventOptions(
              debounced: DebouncedConfig(key: 'thread:x', delay: 100)),
        );
        final u2 = UpdateEvent(
          mutationId: RecordId('_00_pending_mutations', 'm2'),
          recordId: RecordId.parse('thread:x'),
          data: {'title': 'b'},
          beforeRecord: {'title': 'second'},
          options: const PushEventOptions(
              debounced: DebouncedConfig(key: 'thread:x', delay: 100)),
        );
        q.push(u1);
        q.push(u2);
        async.elapse(const Duration(milliseconds: 100));
        expect(q.size, 1);
        // The coalesced event keeps the FIRST beforeRecord.
        expect(u2.beforeRecord, {'title': 'original'});
      });
    });

    test('loadFromDatabase reconstructs events from the outbox', () async {
      db.putMutation('_00_pending_mutations:1', {
        'mutationType': 'create',
        'recordId': 'thread:a',
        'created_at': '2026-01-01T00:00:00.000Z',
      });
      db.putMutation('_00_pending_mutations:2', {
        'mutationType': 'update',
        'recordId': 'thread:b',
        'data': {'title': 'x'},
        'created_at': '2026-01-02T00:00:00.000Z',
      });
      await q.loadFromDatabase();
      expect(q.size, 2);
    });
  });

  group('DownQueue', () {
    test('push/next FIFO; error re-queues', () async {
      final q = DownQueue(logger);
      q.push(RegisterEvent('h1'));
      q.push(SyncDownEvent('h2'));
      expect(q.size, 2);

      final seen = <String>[];
      await q.next((e) async => seen.add(e.hash));
      expect(seen, ['h1']);

      await expectLater(
        q.next((e) async => throw Exception('boom')),
        throwsA(isA<Exception>()),
      );
      expect(q.size, 1); // re-queued
    });
  });

  group('SyncScheduler', () {
    late LocalDatabaseService db;
    setUp(() => db = LocalDatabaseService.open(logger)..provision());
    tearDown(() => db.close());

    test('draining the up-queue then triggers a down-sync', () async {
      final up = UpQueue(db, logger);
      final down = DownQueue(logger);
      final ups = <String>[];
      final downs = <String>[];
      final scheduler = SyncScheduler(
        up,
        down,
        (e) async => ups.add(e.recordId.encode()),
        (e) async => downs.add(e.hash),
        logger,
      );
      await scheduler.init();

      down.push(RegisterEvent('h1')); // queued; down-sync runs (up empty)
      up.push(create('a')); // enqueue -> syncUp drains -> then syncDown
      await Future<void>.delayed(const Duration(milliseconds: 20));

      expect(ups, contains('thread:a'));
      expect(downs, contains('h1'));
    });
  });
}
