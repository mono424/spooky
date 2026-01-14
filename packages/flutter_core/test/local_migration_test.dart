import 'dart:io';
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:flutter_core/core/services/database/local.dart';
import 'package:flutter_core/core/services/database/local_migration.dart';
import 'package:flutter_core/core/types.dart';

void main() {
  test(
    'LocalMigration.provision should apply USER schema and verify functionality',
    () async {
      final dylibPath =
          '/Users/timohty/projekts/spooky/packages/flutter_surrealdb_engine/rust/target/debug/librust_lib_surrealdb.dylib';
      await RustLib.init(externalLibrary: ExternalLibrary.open(dylibPath));

      final config = DatabaseConfig(
        path: ":memory:",
        namespace: "test_ns",
        database: "test_db",
      );

      final localService = await LocalDatabaseService.connect(config);
      await localService.init();

      final migrator = LocalMigration(localService);

      // 1. Read User Schema
      final schemaFile = File(
        '/Users/timohty/projekts/spooky/example/schema/src/schema.surql',
      );
      if (!await schemaFile.exists()) {
        fail("Schema file not found at ${schemaFile.path}");
      }
      final schemaContent = await schemaFile.readAsString();
      print(
        "Loaded schema from ${schemaFile.path} (${schemaContent.length} bytes)",
      );

      // 2. Provision User Schema
      print("Step 1: Running Provisioning with User Schema...");
      await migrator.provision(schemaContent);

      // 3. Verify Schema Application
      print("Step 2: Verifying Schema Application...");

      // Try to create a dummy user to verify 'user' table definition behaves as expected
      // Password must be > 0 chars, username > 3 chars (based on known constraints)
      try {
        await localService.getClient.query(
          sql:
              "CREATE user:test_verify SET username = 'test_verify', password = 'password', created_at = time::now();",
        );
        final res1 = await localService.getClient.query(
          sql: "SELECT * FROM user:test_verify;",
        );
        expect(res1, contains("test_verify"));
        print("✅ Schema Verified (User table functional)");
      } catch (e) {
        print("Warning: Verification insert failed: $e");
        rethrow;
      }

      // 4. Idempotency Check with User Schema
      print("Step 3: Running Idempotency Check...");
      await migrator.provision(schemaContent);

      final res2 = await localService.getClient.query(
        sql: "SELECT * FROM user:test_verify;",
      );
      expect(res2, contains("test_verify"));
      print("✅ Idempotency Verified");

      await localService.close();
    },
  );
}
