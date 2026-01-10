import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

void main() {
  test('Connect to SurrealDB on port 8666', () async {
    await RustLib.init();
    const url = 'ws://127.0.0.1:8666/rpc';
    print('Attempting to connect to $url...');

    try {
      final client = await SurrealDb.connect(
        mode: StorageMode.remote(url: url),
      );
      print('Successfully connected to $url');

      // improved check: try a simple query to ensure protocol is actually working
      try {
        final info = await client.query(sql: 'INFO FOR DB');
        print('Query check successful: $info');
      } catch (qError) {
        print('Connected but query failed (Protocol mismatch?): $qError');
        // We consider this a failure for practical purposes
        throw qError;
      }

      await client.close();
      print('Connection closed.');
    } catch (e) {
      print('Connection failed: $e');
      fail('Failed to connect to $url: $e');
    }
  });
}
