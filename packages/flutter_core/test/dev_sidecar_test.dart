import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_core/flutter_core.dart';
import 'package:flutter_core/core/services/database/local.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'dart:io';

const SURQL_SCHEMA = """
DEFINE TABLE person SCHEMALESS;
""";

void main() {
  test('DevSidecar spawns server and allows connection', () async {
    // 1. Initialize Flutter Rust Bridge (pointing to local build)
    await RustLib.init(
      externalLibrary: ExternalLibrary.open(
        '../flutter_surrealdb_engine/rust/target/debug/librust_lib_surrealdb.dylib',
      ),
    );

    // 2. Setup Config
    final dbPath = '${Directory.current.path}/test_sidecar_db';
    if (Directory(dbPath).existsSync()) {
      Directory(dbPath).deleteSync(recursive: true);
    }

    final config = SpookyConfig(
      schemaSurql: SURQL_SCHEMA,
      schema: 'test_schema',
      database: DatabaseConfig(
        namespace: 'test_ns',
        database: 'test_db',
        path: dbPath,
        // Use a unique port for testing
        devSidecarPort: 16666,
      ),
    );

    // 3. Connect manually to bypass SpookyClient.init's RustLib check
    final local = await LocalDatabaseService.connect(config.database);
    await local.init(); // Selects NS/DB

    // 4. Verify Connection & Data
    // Create data (returns empty string due to RETURN NONE in Rust)
    await local.client!.create(
      resource: "person",
      data: "{\"name\": \"DartSidecar\"}",
    );

    // Query data
    final result = await local.client!.query(
      sql: "SELECT * FROM person",
      vars: null,
    );
    print("Dart Sidecar Query Result: $result");
    expect(result.contains("DartSidecar"), true);

    // 5. Cleanup
    await local.close();

    // Allow process cleanup time
    await Future.delayed(const Duration(seconds: 1));

    if (Directory(dbPath).existsSync()) {
      Directory(dbPath).deleteSync(recursive: true);
    }
  });
}
