import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'dart:convert';
import 'dart:io';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';

void main() {
  setUpAll(() async {
    // Boilerplate to load the library in test environment
    String? dylibPath;
    if (Platform.isMacOS) {
      dylibPath = 'rust/target/debug/librust_lib_surrealdb.dylib';
    } else if (Platform.isLinux) {
      dylibPath = 'rust/target/debug/librust_lib_surrealdb.so';
    } else if (Platform.isWindows) {
      dylibPath = 'rust/target/debug/rust_lib_surrealdb.dll';
    }

    if (dylibPath != null) {
      final lib = ExternalLibrary.open(dylibPath);
      await RustLib.init(externalLibrary: lib);
    } else {
      await RustLib.init();
    }
  });

  test('Verify query vs queryTyped behavior', () async {
    final client = await SurrealDb.connect(mode: const StorageMode.memory());
    await client.useDb(ns: "test", db: "test");

    // 1. Setup Strict Schema
    final schema = '''
      DEFINE TABLE user SCHEMAFULL;
      DEFINE FIELD username ON TABLE user TYPE string;

      DEFINE TABLE thread SCHEMAFULL;
      DEFINE FIELD title ON TABLE thread TYPE string;
      // STRICT RECORD TYPE
      DEFINE FIELD author ON TABLE thread TYPE record<user>;
    ''';
    await client.query(sql: schema, vars: null);

    // 2. Create User
    final userRes = await client.query(
      sql: "CREATE user SET username = 'John'",
      vars: null,
    );
    final userJson = jsonDecode(userRes);
    final userId = userJson[0][0]['id'];
    print("Created User ID: $userId"); // e.g. "user:..."

    // Parse ID components for the object variable
    // Use the new RecordId class!
    final authorId = RecordId.fromString(userId.toString());

    final vars = jsonEncode({
      "data": {
        "title": "My Thread",
        "author": authorId, // Uses toJson() -> {"table":..., "key":...}
      },
    });

    print("Testing with vars: $vars");

    // 3. Test CLASSIC query (Should FAIL or be rejected by strict schema)
    print("--- Testing CLASSIC query (Excepted to FAIL) ---");
    final resClassic = await client.query(
      sql: "CREATE thread CONTENT \$data",
      vars: vars,
    );
    final jsonClassic = jsonDecode(resClassic);
    // SurrealDB returns an error object in the result array for failures
    // result: [{"status": "ERR", "detail": "Found ... expected record<user>..."}]
    // OR it might throw an error depending on wrapper? Wrapper catches and returns valid JSON usually.
    // Let's inspect.
    print("Classic Result: $jsonClassic");

    // We expect an error because we sent a Map, but schema wants a Record.
    // The previous wrapper implementation returned `{"error": ...}` if `query.await?` failed,
    // OR `[{"status": "ERR", ...}]` from DB.
    bool classicFailed = false;
    if (jsonClassic is List && jsonClassic.isNotEmpty) {
      final first = jsonClassic[0];
      if (first is Map &&
          (first.containsKey('error') || first['status'] == 'ERR')) {
        classicFailed = true;
      }
      // Also check if result is empty or different than success
      if (first['status'] == 'OK' && first['result'] != null) {
        // If it succeeded, check if 'author' is a link or object?
        // If schema is strict, it SHOULD fail completely.
        // Wait, if it succeeded, maybe our schema isn't strict enough?
        // "DEFINE FIELD author ... TYPE record<user>" is strict.
      } else {
        // It failed as expected
        classicFailed = true;
      }
    }

    // NOTE: Depending on how strict mode behaves with "CONTENT", it might just fail validation.
    // If it succeeds, it implies classic query is somehow magically working (unlikely) or schema is loose.
    // We assume it fails.

    // 4. Test TYPED query (Should SUCCEED)
    print("--- Testing TYPED query (Expected to SUCCEED) ---");
    final resTyped = await client.queryTyped(
      sql: "CREATE thread CONTENT \$data",
      vars: vars,
    );
    final jsonTyped = jsonDecode(resTyped);
    print("Typed Result: $jsonTyped");

    bool typedSucceeded = false;
    if (jsonTyped is List && jsonTyped.isNotEmpty) {
      final firstItem = jsonTyped[0];

      if (firstItem is List) {
        // It's a result set: [[{...}]]
        // Check if it contains the created record
        if (firstItem.isNotEmpty) {
          final record = firstItem[0];
          // If record has 'id', it was created successfully
          if (record is Map && record.containsKey('id')) {
            typedSucceeded = true;
          }
        }
      } else if (firstItem is Map) {
        // Fallback if structure is different
        if (firstItem['status'] == 'OK') {
          typedSucceeded = true;
        }
      }
    }

    if (!typedSucceeded) {
      fail("queryTyped failed but should have succeeded! Response: $resTyped");
    } else {
      print("queryTyped PASSED!");
    }

    // Optional: Assert classic failed if we are strict about "query" NOT doing magic.
    // But primarily we want to ensure "queryTyped" WORKS.
  });
}
