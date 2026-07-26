import 'package:spooky_core/spooky_core.dart' show RecordId;
import 'package:spooky_core/src/modules/ref_tables.dart';
import 'package:spooky_core/src/modules/sync/sync_utils.dart';
import 'package:test/test.dart';

void main() {
  group('diffRecordVersionArray', () {
    test('detects added / updated / removed', () {
      final diff = diffRecordVersionArray(
        [('thread:a', 1), ('thread:b', 2)],
        [('thread:a', 1), ('thread:b', 3), ('thread:c', 1)],
      );
      expect(diff.added.map((e) => e.id.encode()), ['thread:c']);
      expect(diff.updated.map((e) => e.id.encode()), ['thread:b']);
      expect(diff.removed, isEmpty);
    });

    test('detects removed', () {
      final diff = diffRecordVersionArray(
        [('thread:a', 1), ('thread:b', 2)],
        [('thread:a', 1)],
      );
      expect(diff.removed.map((e) => e.encode()), ['thread:b']);
    });
  });

  group('ArraySyncer', () {
    test('nextSet diffs local against remote', () {
      final syncer = ArraySyncer(
        [('thread:a', 1)],
        [('thread:a', 1), ('thread:b', 1)],
      );
      final diff = syncer.nextSet()!;
      expect(diff.added.map((e) => e.id.encode()), ['thread:b']);
    });
  });

  group('createDiffFromDbOp', () {
    test('CREATE yields an added entry', () {
      final diff = createDiffFromDbOp('CREATE', RecordId('thread', 'a'), 1, []);
      expect(diff.added.single.id.encode(), 'thread:a');
    });
    test('stale version yields empty diff', () {
      final diff = createDiffFromDbOp(
          'UPDATE', RecordId('thread', 'a'), 1, [('thread:a', 2)]);
      expect(diff.added, isEmpty);
      expect(diff.updated, isEmpty);
      expect(diff.removed, isEmpty);
    });
  });

  group('listRefPollDelayMs', () {
    test('no idle streak -> base interval', () {
      expect(listRefPollDelayMs(idleStreak: 0, baseIntervalMs: 500), 500);
      expect(listRefPollDelayMs(idleStreak: -1, baseIntervalMs: 500), 500);
    });
    test('backs off exponentially while idle', () {
      expect(listRefPollDelayMs(idleStreak: 1, baseIntervalMs: 500), 1000);
      expect(listRefPollDelayMs(idleStreak: 2, baseIntervalMs: 500), 2000);
      expect(listRefPollDelayMs(idleStreak: 3, baseIntervalMs: 500), 4000);
    });
    test('caps at the max interval', () {
      expect(listRefPollDelayMs(idleStreak: 4, baseIntervalMs: 500),
          listRefPollMaxIntervalMs);
      expect(listRefPollDelayMs(idleStreak: 500, baseIntervalMs: 500),
          listRefPollMaxIntervalMs);
    });
    test('an aggressive base is never widened below itself', () {
      // A base above the cap wins: the helper must not slow a deliberately
      // aggressive configuration.
      expect(listRefPollDelayMs(idleStreak: 0, baseIntervalMs: 8000), 8000);
      expect(listRefPollDelayMs(idleStreak: 9, baseIntervalMs: 8000), 8000);
    });
  });

  group('recordVersionArraysEqual', () {
    test('order-insensitive equality', () {
      expect(
        recordVersionArraysEqual(
            [('thread:a', 1), ('thread:b', 2)], [('thread:b', 2), ('thread:a', 1)]),
        isTrue,
      );
    });
    test('detects a version bump, an add, and a removal', () {
      expect(
        recordVersionArraysEqual([('thread:a', 1)], [('thread:a', 2)]),
        isFalse,
      );
      expect(
        recordVersionArraysEqual([('thread:a', 1)], [('thread:a', 1), ('thread:b', 1)]),
        isFalse,
      );
      expect(recordVersionArraysEqual([('thread:a', 1)], []), isFalse);
    });
    test('two empty arrays are equal', () {
      expect(recordVersionArraysEqual([], []), isTrue);
    });
  });

  group('nextPollDelayMs (deprecated, superseded by listRefPollDelayMs)', () {
    test('no LIVE event -> base interval', () {
      expect(
        nextPollDelayMs(now: 1000, lastLiveEventAt: null, baseIntervalMs: 500),
        500,
      );
    });
    test('recent LIVE event -> healthy (slower) interval', () {
      expect(
        nextPollDelayMs(now: 1000, lastLiveEventAt: 900, baseIntervalMs: 500),
        5000,
      );
    });
    test('stale LIVE event -> base interval', () {
      expect(
        nextPollDelayMs(now: 10000, lastLiveEventAt: 900, baseIntervalMs: 500),
        500,
      );
    });
  });

  group('resolveListRefPollInterval', () {
    test('non-positive falls back to default', () {
      expect(resolveListRefPollInterval(0), defaultListRefPollIntervalMs);
      expect(resolveListRefPollInterval(-5), defaultListRefPollIntervalMs);
      expect(resolveListRefPollInterval(250), 250);
    });
  });

  group('ref tables', () {
    test('dedicated mode routes to per-user table', () {
      expect(listRefTableFor(RefMode.dedicated, 'user:abc'),
          '_00_list_ref_user_abc');
      expect(listRefTableFor(RefMode.single, 'user:abc'), '_00_list_ref');
      expect(listRefTableFor(RefMode.dedicated, null), '_00_list_ref');
      expect(listRefTableFor(RefMode.dedicated, 'user:bad-id'), '_00_list_ref');
    });

    test('sanitizeUserId strips prefix and validates', () {
      expect(sanitizeUserId('user:abc'), 'abc');
      expect(sanitizeUserId('abc'), 'abc');
      expect(sanitizeUserId('user:a-b'), isNull);
      expect(sanitizeUserId(''), isNull);
    });
  });
}
