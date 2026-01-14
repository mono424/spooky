import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_core/flutter_core.dart';

void main() {
  test('SpookyConfig initialization', () {
    const config = SpookyConfig(
      schemaString: 'DEFINE TABLE user SCHEMAFULL;',
      globalDBurl: 'wss://cloud.surrealdb.com/rpc',
      localDBurl: 'indxdb://spooky',
      dbName: 'spooky',
      namespace: 'test',
      database: 'test',
      internalDatabase: 'spooky_internal',
    );

    expect(config.dbName, 'spooky');
    expect(config.globalDBurl, 'wss://cloud.surrealdb.com/rpc');
  });

  // Note: Testing Spooky.create() requires mocking the underlying flutter_surrealdb_engine
  // which likely involves FFI or Platform Channels.
  // For this porting task, static verification via compilation of this test file is the primary goal.
}
