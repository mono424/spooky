import './database.dart';
import '../../types.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

class LocalDatabaseService extends AbstractDatabaseService {
  final DatabaseConfig config;

  //LocalDatabaseService._(SurrealDatabase client) : super(client);
  LocalDatabaseService._(super.client, this.config);

  static Future<LocalDatabaseService> connect(DatabaseConfig config) async {
    final client = await connectDb(path: config.path);
    return LocalDatabaseService._(client, config);
  }

  DatabaseConfig get getConfig => config;

  @override
  Future<void> init() async {
    try {
      await client!.useNs(ns: config.namespace);
      await client!.useDb(db: config.database);
    } catch (e) {
      throw Exception('Error setting local NS/DB: $e');
    }
  }
}
