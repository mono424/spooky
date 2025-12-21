import './database.dart';
import '../../types.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

class RemoteDatabaseService extends AbstractDatabaseService {
  final DatabaseConfig config;

  RemoteDatabaseService._(super.client, this.config);

  static Future<RemoteDatabaseService> connect(DatabaseConfig config) async {
    SurrealDatabase? client;
    if (config.endpoint != null) {
      client = await connectDb(path: config.endpoint!);
    }
    return RemoteDatabaseService._(client, config);
  }

  DatabaseConfig get getConfig => config;

  @override
  Future<void> init() async {
    if (client == null) return;
    try {
      await client!.useNs(ns: config.namespace);
      await client!.useDb(db: config.database);
      if (config.token != null) {
        await client!.authenticate(token: config.token!);
      }
    } catch (e) {
      throw Exception('Error setting remote NS/DB: $e');
    }
  }
}
