import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

import 'types.dart';
import 'services/database/local.dart';
import 'services/database/remote.dart';
import 'services/database/local_migration.dart';
export 'types.dart';

class SpookyClient {
  final SpookyConfig config;
  final LocalDatabaseService local;
  final RemoteDatabaseService remote;
  final LocalMigration migrator;

  SpookyClient._(this.config, this.local, this.remote, this.migrator);

  static bool _rustInitialized = false;

  static Future<SpookyClient> init(SpookyConfig config) async {
    if (!_rustInitialized) {
      await RustLib.init();
      _rustInitialized = true;
    }
    final local = await LocalDatabaseService.connect(config.database);
    await local.init();
    final remote = await RemoteDatabaseService.connect(config.database);
    await remote.init();
    final migrator = LocalMigration(local);
    await migrator.provision(config.schemaSurql);

    return SpookyClient._(config, local, remote, migrator);
  }

  Future<void> close() async {
    await local.close();
    await remote.close();
  }
}
