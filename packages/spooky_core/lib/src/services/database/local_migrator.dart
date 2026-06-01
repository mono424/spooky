import 'dart:convert';

import 'package:crypto/crypto.dart';

import '../logger/logger.dart';
import 'local_database_service.dart';

/// SHA-1 hex of the schema text (TS `sha1`). Used to detect schema changes.
String schemaSha1(String schemaSurql) =>
    sha1.convert(utf8.encode(schemaSurql)).toString();

/// Provisions the local store against a schema and migrates on change
/// (TS `LocalMigrator`).
///
/// The JS migrator runs `DEFINE/REMOVE DATABASE` against SurrealDB; the sqlite
/// store needs no per-table DDL (records are document-style JSON), so the
/// faithful mapping is: hash the schema, and on a hash change wipe stale local
/// data (so it can't conflict with the new schema), then record the new hash.
/// Unchanged schema is a no-op.
class LocalMigrator {
  LocalMigrator(this._local, SpookyLogger logger)
      : _logger = logger.child('LocalMigrator');

  final LocalDatabaseService _local;
  final SpookyLogger _logger;

  Future<void> provision(String schemaSurql) async {
    final hash = schemaSha1(schemaSurql);

    if (_local.latestSchemaHash() == hash) {
      _logger.info('[Provisioning] Schema up to date, skipping migration');
      return;
    }

    _logger.info('[Provisioning] Schema changed, resetting local data');
    _local.resetLocalData();
    _local.recordSchemaHash(hash, DateTime.now().toUtc().toIso8601String());
  }
}
