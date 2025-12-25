import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

abstract class AbstractDatabaseService {
  final SurrealDb? client;

  AbstractDatabaseService(this.client);

  SurrealDb get getClient => client!;

  Future<void> init();

  Future<void> close() async {
    client?.close();
  }
}
