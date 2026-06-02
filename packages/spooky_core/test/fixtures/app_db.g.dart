// GENERATED CODE - DO NOT EDIT BY HAND.
// Generated from a SurrealQL schema by spooky_core codegen.

import 'package:spooky_core/spooky_core.dart';
import 'package:spooky_core/typed.dart';

final spookySchema = <String, dynamic>{
  'thread': {
    'columns': {
      'title': ColumnSchema(type: 'string'),
      'score': ColumnSchema(type: 'int'),
      'published': ColumnSchema(type: 'bool'),
    },
  },
};

const surqlSchema = r'''
DEFINE TABLE thread SCHEMAFULL PERMISSIONS FOR select WHERE true;
DEFINE FIELD title ON thread TYPE string;
DEFINE FIELD score ON thread TYPE int;
DEFINE FIELD published ON thread TYPE bool;
''';

class Thread {
  Thread({
    required this.id,
    required this.title,
    required this.score,
    required this.published,
  });

  final String id;
  final String title;
  final int score;
  final bool published;

  factory Thread.fromJson(Map<String, dynamic> json) => Thread(
        id: json['id'] as String,
        title: json['title'] as String,
        score: json['score'] as int,
        published: json['published'] as bool,
      );

  Map<String, dynamic> toJson() => {
        'id': id,
        'title': title,
        'score': score,
        'published': published,
      };
}

class ThreadPatch {
  ThreadPatch({
    this.title,
    this.score,
    this.published,
  });

  final String? title;
  final int? score;
  final bool? published;

  Map<String, dynamic> toJson() {
    final m = <String, dynamic>{};
    if (title != null) m['title'] = title;
    if (score != null) m['score'] = score;
    if (published != null) m['published'] = published;
    return m;
  }
}

abstract final class Thread$ {
  static const id = StringField('id');
  static const title = StringField('title');
  static const score = NumField('score');
  static const published = BoolField('published');
}

class ThreadCollection {
  ThreadCollection(this._c);
  final Sp00kyClient _c;

  TypedQuery<Thread> query() => TypedQuery(_c.query('thread'), Thread.fromJson);

  Future<void> create(Thread model) =>
      _c.create(model.id, model.toJson()..remove('id'));

  Future<void> update(String id, ThreadPatch patch) =>
      _c.update('thread', id, patch.toJson());

  Future<void> delete(String id) => _c.delete('thread', id);
}

class AppDb {
  AppDb(this.client);
  final Sp00kyClient client;

  factory AppDb.open(DatabaseConfig database) => AppDb(
        Sp00kyClient(Sp00kyConfig(
          database: database,
          schema: spookySchema,
          schemaSurql: surqlSchema,
        )),
      );

  Future<void> init() => client.init();
  Future<void> close() => client.close();
  ThreadCollection get thread => ThreadCollection(client);
}
