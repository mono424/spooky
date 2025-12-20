import 'dart:convert';
import 'package:crypto/crypto.dart' as crypto;
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'backup/db.dart';
import 'services/logger.dart';

// Assuming Logger interface exists or we create a simple one
// For now, using a placeholder if Logger is not found

class ProvisionOptions {
  final bool force;
  ProvisionOptions({this.force = false});
}

class SchemaRecord {
  final String hash;
  final String createdAt;

  SchemaRecord({required this.hash, required this.createdAt});

  factory SchemaRecord.fromJson(Map<String, dynamic> json) {
    return SchemaRecord(
      hash: json['hash'] as String,
      createdAt: json['created_at'] as String,
    );
  }
}

class ProvisionContext {
  final SurrealDatabase internalDb;
  final SurrealDatabase localDb;
  final String namespace;
  final String database;
  final String internalDatabase;
  final String schema;

  ProvisionContext({
    required this.internalDb,
    required this.localDb,
    required this.namespace,
    required this.database,
    required this.internalDatabase,
    required this.schema,
  });
}

Future<String> sha1(String input) async {
  var bytes = utf8.encode(input);
  var digest = crypto.sha1.convert(bytes);
  return digest.toString();
}

Future<void> initializeInternalDatabase(SurrealDatabase internalDb) async {
  await internalDb.queryDb(
    query: r'''
    DEFINE TABLE IF NOT EXISTS __schema SCHEMAFULL;
    DEFINE FIELD IF NOT EXISTS id ON __schema TYPE string;
    DEFINE FIELD IF NOT EXISTS hash ON __schema TYPE string;
    DEFINE FIELD IF NOT EXISTS created_at ON __schema TYPE datetime VALUE time::now();
    DEFINE INDEX IF NOT EXISTS unique_hash ON __schema FIELDS hash UNIQUE;
  ''',
  );
}

Future<bool> isSchemaUpToDate(SurrealDatabase internalDb, String hash) async {
  try {
    final results = await internalDb.queryDb(
      query:
          r'SELECT hash, created_at FROM __schema ORDER BY created_at DESC LIMIT 1;',
    );

    if (results.isEmpty || results.first.result == null) return false;

    final parsed = jsonDecode(results.first.result!);
    if (parsed is List && parsed.isNotEmpty) {
      // parsed[0] is the row object
      final row = parsed[0];
      final storedHash = row['hash'];
      return storedHash == hash;
    }

    return false;
  } catch (e) {
    return false;
  }
}

Future<void> dropMainDatabase(SurrealDatabase localDb, String database) async {
  try {
    await localDb.queryDb(query: 'REMOVE DATABASE $database;');
  } catch (e) {
    // Ignore
  }
  await localDb.queryDb(query: 'DEFINE DATABASE $database;');
}

Future<void> provisionSchema(
  SurrealDatabase localDb,
  String schemaContent,
) async {
  final statements = schemaContent
      .split(';')
      .map((s) => s.trim())
      .where((s) => s.isNotEmpty);
  //for (final statement in statements) {
  await localDb.queryDb(query: schemaContent);
  //}
}

Future<void> recordSchemaHash(SurrealDatabase internalDb, String hash) async {
  // We need to use interpolation for the hash value
  // Assuming a simple way to replace $hash.
  // In a real scenario, use parameterized queries if available or careful interpolation.

  // Using jsonEncode to ensure it's quoted properly as a string
  final hashStr = jsonEncode(hash);

  await internalDb.queryDb(
    query:
        '''
      UPSERT __schema SET hash = $hashStr, created_at = time::now() WHERE hash = $hashStr;
    ''',
  );
}

Future<void> runProvision(
  String database,
  String schemaSurql,
  DatabaseService databaseService,
  Logger logger, {
  ProvisionOptions? options,
}) async {
  final opts = options ?? ProvisionOptions();

  logger.info("[Provisioning] Starting provision check...");

  // Result class or map
  final result = await databaseService.runWithInternal((db) async {
    final schemaHash = await sha1(schemaSurql);
    final isUpToDate = await isSchemaUpToDate(db, schemaHash);
    final shouldMigrate = opts.force || !isUpToDate;

    return {
      'shouldMigrate': shouldMigrate,
      'schemaHash': schemaHash,
      'isUpToDate': isUpToDate,
    };
  });

  final shouldMigrate = result['shouldMigrate'] as bool;
  final schemaHash = result['schemaHash'] as String;
  final isUpToDate = result['isUpToDate'] as bool;

  logger.debug("[Provisioning] Schema hash: $schemaHash");
  logger.debug("[Provisioning] Schema up to date: $isUpToDate");
  logger.debug("[Provisioning] Should migrate: $shouldMigrate");

  if (!shouldMigrate) {
    logger.info("[Provisioning] Schema is up to date, skipping migration");
    return;
  }

  logger.info("[Provisioning] Initializing internal database schema...");
  await databaseService.runWithInternal((db) async {
    await initializeInternalDatabase(db);
  });

  logger.info("[Provisioning] Starting schema migration...");
  await databaseService.runWithLocal((db) async {
    await dropMainDatabase(db, database);
    await provisionSchema(db, schemaSurql);
  });

  logger.debug("[Provisioning] Recording schema hash...");
  await databaseService.runWithInternal((db) async {
    await recordSchemaHash(db, schemaHash);
  });

  logger.info("[Provisioning] Database schema provisioned successfully");
}
