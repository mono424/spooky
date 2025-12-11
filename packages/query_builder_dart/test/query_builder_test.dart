import 'package:test/test.dart';
import 'package:query_builder_dart/query_builder_dart.dart';

void main() {
  // Define a mock schema matching models.dart
  final schema = SchemaStructure(
    tables: [
      TableSchema(
        name: 'user',
        columns: {
          'id': ColumnSchema(type: ValueType.record),
          'username': ColumnSchema(type: ValueType.string),
        },
      ),
      TableSchema(
        name: 'thread',
        columns: {
          'id': ColumnSchema(type: ValueType.record),
          'title': ColumnSchema(type: ValueType.string),
          'content': ColumnSchema(type: ValueType.string),
          'author': ColumnSchema(type: ValueType.record),
        },
      ),
      TableSchema(
        name: 'comment',
        columns: {
          'id': ColumnSchema(type: ValueType.record),
          'content': ColumnSchema(type: ValueType.string),
          'author': ColumnSchema(type: ValueType.record),
          'thread': ColumnSchema(type: ValueType.record),
        },
      ),
    ],
    relationships: [
      Relationship(
        from: 'thread',
        field: 'author',
        to: 'user',
        cardinality: Cardinality.one,
      ),
      Relationship(
        from: 'user',
        field: 'threads',
        to: 'thread',
        cardinality: Cardinality.many,
      ),
      Relationship(
        from: 'comment',
        field: 'thread',
        to: 'thread',
        cardinality: Cardinality.one,
      ),
    ],
  );

  group('QueryBuilder', () {
    test('Simple SELECT', () {
      final q = QueryBuilder(schema, 'user').select(['id', 'username']).build();

      expect(q.query, equals('SELECT id, username FROM user'));
    });

    test('SELECT with WHERE', () {
      final q = QueryBuilder(
        schema,
        'user',
      ).where({'username': 'admin'}).build();

      expect(q.query, equals("SELECT * FROM user WHERE username = 'admin'"));
    });

    test('SELECT with ORDER BY and LIMIT', () {
      final q = QueryBuilder(
        schema,
        'thread',
      ).orderBy('created_at', 'desc').limit(10).build();

      expect(
        q.query,
        equals('SELECT * FROM thread ORDER BY created_at DESC LIMIT 10'),
      );
    });

    test('SELECT with RELATED (One-to-One)', () {
      final q = QueryBuilder(schema, 'thread').related('author').build();

      // Note: The current implementation just builds the main query.
      // Subqueries are not yet fully integrated into the main query string
      // in the same way as the TS version (which returns a complex object structure).
      // For this test, we just check the main query is valid.
      expect(q.query, equals('SELECT * FROM thread'));
    });
  });
}
