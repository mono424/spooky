import 'package:spooky_core/spooky_core.dart';
import 'package:test/test.dart';

import 'fixtures/app_db.g.dart';

/// Drives a *generated* AppDb over a real local Sp00kyClient (FFI), proving the
/// typed surface works at runtime: typed create -> typed where -> typed
/// Stream<List<Thread>>, and typed update.
void main() {
  late AppDb db;

  setUp(() async {
    db = AppDb.open(const DatabaseConfig(namespace: 't', database: 't'));
    await db.init();
  });
  tearDown(() => db.close());

  test('typed create + typed filtered watch yields typed models', () async {
    final emissions = <List<Thread>>[];
    final sub = db.thread
        .query()
        .where([Thread$.published.eq(true), Thread$.score.gte(10)])
        .orderBy(Thread$.score, desc: true)
        .watch()
        .listen(emissions.add);
    await Future<void>.delayed(const Duration(milliseconds: 20));

    // Two published rows (one below the score threshold) + one unpublished.
    await db.thread.create(
        Thread(id: 'thread:a', title: 'hi', score: 20, published: true));
    await db.thread.create(
        Thread(id: 'thread:b', title: 'low', score: 5, published: true));
    await db.thread.create(
        Thread(id: 'thread:c', title: 'draft', score: 99, published: false));
    await Future<void>.delayed(const Duration(milliseconds: 80));

    final latest = emissions.last;
    expect(latest, isA<List<Thread>>());
    expect(
        latest.map((t) => t.id), ['thread:a']); // only published && score>=10
    expect(latest.single.title, 'hi');
    await sub.cancel();
  });

  test('typed update via ThreadPatch reflects in a typed watchOne', () async {
    await db.thread
        .create(Thread(id: 'thread:x', title: 'v1', score: 1, published: true));

    final seen = <Thread?>[];
    final sub = db.thread
        .query()
        .where([Thread$.id.eq('thread:x')])
        .watchOne()
        .listen(seen.add);
    await Future<void>.delayed(const Duration(milliseconds: 30));

    await db.thread.update('thread:x', ThreadPatch(title: 'v2'));
    await Future<void>.delayed(const Duration(milliseconds: 120));

    expect(seen.last, isA<Thread>());
    expect(seen.last!.title, 'v2');
    await sub.cancel();
  });
}
