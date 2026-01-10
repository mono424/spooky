import 'dart:io';
import 'package:flutter_core/core/services/database/local.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_core/core/types.dart';

void main() {
  test('Persistence Verification Test', () async {
    // 1. Setup path
    final dbPath =
        '${Directory.current.path}/test_db_persistence_${DateTime.now().millisecondsSinceEpoch}';
    print("Testing persistence at: $dbPath");

    final config = DatabaseConfig(
      namespace: 'test_ns',
      database: 'test_db',
      path: dbPath,
    );

    // 2. Open, Write, Close
    {
      print("Phase 1: Open and Write");
      await RustLib.init(
        externalLibrary: ExternalLibrary.open(
          '/Users/timohty/projekts/spooky/packages/flutter_surrealdb_engine/rust/target/debug/librust_lib_surrealdb.dylib',
        ),
      );
      final local = await LocalDatabaseService.connect(config);
      await local.init();

      // Create user
      await local.getClient.query(
        sql: "CREATE user:test_user SET username = 'persistent_guy'",
      );

      // Verify in memory
      final res1 = await local.getClient.query(
        sql: "SELECT * FROM user:test_user",
      );
      print("Write Result: $res1");
      expect(res1.contains('persistent_guy'), true);

      // Force Flush via Export (our fix)
      final dumpPath = '$dbPath/dump.surql';
      await local.export(dumpPath);

      await local.close();
      print("Phase 1: Closed");

      // Give it a moment to release locks
      await Future.delayed(const Duration(seconds: 1));
    }

    // 3. Re-Open, Read
    {
      print("Phase 2: Re-Open and Read");
      final local = await LocalDatabaseService.connect(config);
      await local.init();

      final res2 = await local.getClient.query(
        sql: "SELECT * FROM user:test_user",
      );
      print("Read Result (After Restart): $res2");

      await local.close();

      // Assert
      if (!res2.contains('persistent_guy')) {
        fail("Data did not persist! Result: $res2");
      }
    }

    // Cleanup
    if (Directory(dbPath).existsSync()) {
      Directory(dbPath).deleteSync(recursive: true);
    }
  });
}
