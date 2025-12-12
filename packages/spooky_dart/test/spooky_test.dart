import 'package:test/test.dart';
import 'package:spooky_dart/spooky_dart.dart';

void main() {
  // Mock schema
  final schema = SchemaStructure(
    tables: [TableSchema(name: 'user', columns: {})],
    relationships: [],
  );

  group('SpookyInstance', () {
    // Note: We cannot easily test the full instance without a real database connection
    // or a mock engine. Since we are using the real engine package which requires FFI,
    // running this test in a pure Dart environment might fail if the library isn't loaded.
    // However, we can test the structure and compilation.

    test('Configuration', () {
      final config = SpookyConfig(databasePath: 'memory', schema: schema);

      expect(config.databasePath, equals('memory'));
      expect(config.schema, equals(schema));
    });
  });
}
