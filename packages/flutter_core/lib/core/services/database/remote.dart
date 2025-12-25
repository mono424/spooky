import './database.dart';
import '../../types.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

class RemoteDatabaseService extends AbstractDatabaseService {
  final DatabaseConfig config;

  RemoteDatabaseService._(super.client, this.config);

  static Future<RemoteDatabaseService> connect(DatabaseConfig config) async {
    SurrealDb? client;
    if (config.endpoint != null) {
      try {
        client = await SurrealDb.connect(
          mode: StorageMode.remote(url: config.endpoint!),
        );
      } catch (e) {
        // Rethrow the error so it can be seen in the UI logs for debugging
        throw Exception(
          'Failed to connect to remote SurrealDB at ${config.endpoint}: $e',
        );
      }
    }
    return RemoteDatabaseService._(client, config);
  }

  DatabaseConfig get getConfig => config;

  @override
  Future<void> init() async {
    if (client == null) return;
    try {
      await client!.useDb(
        namespace: config.namespace,
        database: config.database,
      );
      if (config.token != null) {
        await client!.authenticate(token: config.token!);
      }
    } catch (e) {
      throw Exception('Error setting remote NS/DB: $e');
    }
  }
}
