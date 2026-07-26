import 'package:spooky_core/codegen.dart';
import 'package:test/test.dart';

const _surql = '''
DEFINE TABLE user SCHEMAFULL PERMISSIONS FOR select WHERE id = \$auth.id;
DEFINE FIELD username ON user TYPE string ASSERT \$value != NONE;
DEFINE FIELD created_at ON user TYPE option<datetime> DEFAULT time::now();
DEFINE TABLE thread SCHEMAFULL PERMISSIONS FOR select WHERE true;
DEFINE FIELD title ON thread TYPE string;
DEFINE FIELD author ON TABLE thread TYPE record<user>;
DEFINE FIELD score ON thread TYPE int;
DEFINE FIELD tags ON thread TYPE array<string>;
''';

void main() {
  group('parseSchema', () {
    test('extracts tables and fields with types/flags', () {
      final tables = parseSchema(_surql);
      expect(tables.map((t) => t.name), ['user', 'thread']);

      final user = tables.firstWhere((t) => t.name == 'user');
      final username = user.fields.firstWhere((f) => f.name == 'username');
      expect(username.type, 'string');
      expect(username.optional, isFalse);

      final createdAt = user.fields.firstWhere((f) => f.name == 'created_at');
      expect(createdAt.isDateTime, isTrue);
      expect(createdAt.optional, isTrue);

      final thread = tables.firstWhere((t) => t.name == 'thread');
      final author = thread.fields.firstWhere((f) => f.name == 'author');
      expect(author.isRecord, isTrue);
      expect(author.recordTable, 'user');

      final tags = thread.fields.firstWhere((f) => f.name == 'tags');
      expect(tags.type, 'array<string>');
    });
  });

  group('relationships', () {
    test('the emitted schema map carries forward + reverse relationships', () {
      final src = emitSchemaMap(parseSchema(_surql));
      expect(src, contains("'relationships': ["));
      expect(
        src,
        contains(
            "{'from': 'thread', 'field': 'author', 'to': 'user', 'cardinality': 'one'}"),
      );
      expect(
        src,
        contains(
            "{'from': 'user', 'field': 'threads', 'to': 'thread', 'cardinality': 'many'}"),
        reason: 'the reverse name is the pluralized source table',
      );
    });

    test('a schema with no record fields emits no relationships key', () {
      final src = emitSchemaMap(parseSchema(
          'DEFINE TABLE thread SCHEMAFULL;\nDEFINE FIELD title ON thread TYPE string;'));
      expect(src, isNot(contains('relationships')));
    });
  });

  group('emitSchemaMap', () {
    test('produces a usable ColumnSchema map literal', () {
      final src = emitSchemaMap(parseSchema(_surql));
      expect(src, contains("'user':"));
      expect(src, contains("'thread':"));
      expect(src, contains("'username': ColumnSchema(type: 'string')"));
      expect(
        src,
        contains(
            "'created_at': ColumnSchema(type: 'datetime', dateTime: true, optional: true)"),
      );
      expect(src,
          contains("'author': ColumnSchema(type: 'record', recordId: true)"));
    });
  });

  group('emitModels', () {
    test('emits a PascalCase class per table with typed fields', () {
      final src = emitModels(parseSchema(_surql));
      expect(src, contains('class User {'));
      expect(src, contains('class Thread {'));
      expect(src, contains('final String username;'));
      expect(src, contains('final DateTime? created_at;'));
      expect(src, contains('final String author;')); // record id as string
      expect(src, contains('final int score;'));
      expect(src, contains('final List<String> tags;'));
      expect(
          src, contains('factory Thread.fromJson(Map<String, dynamic> json)'));
      expect(src, contains('Map<String, dynamic> toJson()'));
    });
  });

  group('generateDartSource', () {
    test('is analyzable: imports + schema map + models', () {
      final src = generateDartSource(_surql);
      expect(src, contains("import 'package:spooky_core/spooky_core.dart';"));
      expect(src, contains('final spookySchema = <String, dynamic>{'));
      expect(src, contains('class User {'));
    });
  });
}
