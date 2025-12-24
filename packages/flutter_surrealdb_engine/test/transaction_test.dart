
import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

void main() {
  test('SurrealTransaction commit test', () async {
    // Initialize Rust FFI.
    // Note: This requires the dynamic library to be available in the test environment (e.g. DYLD_LIBRARY_PATH on macOS).
    try {
      await RustLib.init();
    } catch (e) {
      // If init fails (e.g. library not found during unit test), skipping might be appropriate
      // or we fail. For now, let's print and assume environment execution.
      print('Warning: RustLib init failed: $e. Test might not work if FFI is not linked.');
    }

    final db = await connectDb(path: 'memory');
    await db.useNs(ns: 'test');
    await db.useDb(db: 'test');

    // Start transaction
    final tx = await db.beginTransaction();
    await tx.query(query: 'CREATE person:1 SET name = "Alice"');
    
    // Commit
    await tx.commit();

    // Verify
    final result = await db.queryDb(query: 'SELECT * FROM person:1');
    expect(result.isNotEmpty, true);
    // Result string depends on serialization, but should contain Alice
    expect(result.first.result, contains('Alice'));
  });

  test('SurrealTransaction cancel test', () async {
    try {
      await RustLib.init();
    } catch (e) {/* ignore if already inited */}

    final db = await connectDb(path: 'memory');
    await db.useNs(ns: 'test');
    await db.useDb(db: 'test');

    // Start transaction
    final tx = await db.beginTransaction();
    await tx.query(query: 'CREATE person:2 SET name = "Bob"');
    
    // Cancel
    await tx.cancel();

    // Verify
    final result = await db.queryDb(query: 'SELECT * FROM person:2');
    // Should be empty or null result
    if (result.isNotEmpty && result.first.result != null) {
       // Check if it's strictly empty array "[]" or null
       final resStr = result.first.result!;
       if (resStr != "[]") {
           fail('Expected empty result, got $resStr');
       }
    }
  });
}
