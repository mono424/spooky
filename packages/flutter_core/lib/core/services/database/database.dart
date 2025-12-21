import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

abstract class AbstractDatabaseService {
  final SurrealDatabase? client;

  AbstractDatabaseService(this.client);

  SurrealDatabase get getClient => client!;

  Future<void> init();

  Future<void> close() async {
    await client?.invalidate();
  }
}
