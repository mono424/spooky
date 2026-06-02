import 'dart:async';

import 'package:spooky_core/spooky_core.dart' show QueryBuilder, RecordId;
import 'package:spooky_core/typed.dart';
import 'package:test/test.dart';

/// A toy model standing in for a generated one.
class Thread {
  Thread(this.id, this.title);
  final String id;
  final String title;
  factory Thread.fromJson(Map<String, dynamic> j) =>
      Thread(j['id'] as String, j['title'] as String);
}

/// Field tokens a generator would emit for `thread`.
abstract final class Thread$ {
  static const id = StringField('id');
  static const title = StringField('title');
  static const score = NumField('score');
  static const published = BoolField('published');
  static const author = RecordField('author');
  static const createdAt = DateTimeField('created_at');
}

void main() {
  // Capture what the underlying builder registers.
  late String capturedSql;
  late Map<String, dynamic> capturedVars;
  late StreamController<List<Map<String, dynamic>>> feed;

  QueryBuilder builder() => QueryBuilder(
        'thread',
        registrar: (sql, vars, ttl) async {
          capturedSql = sql;
          capturedVars = vars;
          return 'hash1';
        },
        subscriber: (hash) => feed.stream,
      );

  TypedQuery<Thread> typed() => TypedQuery(builder(), Thread.fromJson);

  setUp(() => feed = StreamController<List<Map<String, dynamic>>>.broadcast());
  tearDown(() => feed.close());

  test('typed where + orderBy compiles to the right SURQL + vars', () async {
    await typed()
        .where([Thread$.published.eq(true), Thread$.score.gt(10)])
        .orderBy(Thread$.createdAt, desc: true)
        .limit(5)
        .run();

    expect(
      capturedSql,
      'SELECT * FROM thread WHERE published = \$published AND score > \$score '
      'ORDER BY created_at DESC LIMIT 5;',
    );
    expect(capturedVars, {'published': true, 'score': 10});
  });

  test('eq uses a plain value; comparisons use QueryOp', () {
    expect(Thread$.title.eq('x').op, isNull);
    expect(Thread$.score.gt(3).op, '>');
    expect(Thread$.score.lte(9).op, '<=');
  });

  test('RecordField.eq accepts a RecordId or string', () async {
    await typed().where([Thread$.author.eq(RecordId('user', 'u1'))]).run();
    expect(capturedVars['author'], isA<RecordId>());
  });

  test('watch maps rows to typed models', () async {
    final emissions = <List<Thread>>[];
    final sub = typed()
        .where([Thread$.published.eq(true)])
        .watch()
        .listen(emissions.add);
    await Future<void>.delayed(Duration.zero);

    feed.add([
      {'id': 'thread:a', 'title': 'hello'},
      {'id': 'thread:b', 'title': 'world'},
    ]);
    await Future<void>.delayed(Duration.zero);

    expect(emissions.last, isA<List<Thread>>());
    expect(emissions.last.map((t) => t.title), ['hello', 'world']);
    await sub.cancel();
  });

  test('watchOne sets LIMIT 1 and maps to first-or-null', () async {
    final got = <Thread?>[];
    final sub =
        typed().where([Thread$.id.eq('thread:a')]).watchOne().listen(got.add);
    await Future<void>.delayed(Duration.zero);

    feed.add([]); // empty -> null
    feed.add([
      {'id': 'thread:a', 'title': 'solo'}
    ]);
    await Future<void>.delayed(Duration.zero);

    expect(got.first, isNull);
    expect(got.last!.title, 'solo');
    expect(capturedSql, contains('LIMIT 1'));
    await sub.cancel();
  });
}
