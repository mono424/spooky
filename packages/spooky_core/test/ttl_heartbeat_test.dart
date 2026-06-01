import 'package:spooky_core/src/modules/cache/cache_module.dart';
import 'package:spooky_core/src/modules/data/data_module.dart';
import 'package:spooky_core/src/services/database/local_database_service.dart';
import 'package:spooky_core/src/services/logger/logger.dart';
import 'package:spooky_core/src/services/persistence/memory_persistence.dart';
import 'package:spooky_core/src/services/stream_processor/stream_processor_service.dart';
import 'package:test/test.dart';

/// Builds a real local DataModule stack (sqlite + FFI processor + cache).
Future<(DataModule, void Function())> _buildDataModule(
  SpookyLogger logger, {
  void Function(String hash)? onHeartbeat,
}) async {
  final local = LocalDatabaseService.open(logger)..provision();
  final sp = StreamProcessorService(MemoryPersistenceClient(), logger);
  await sp.init();
  sp.seedPermissionsFromSchema(
      'DEFINE TABLE thread SCHEMAFULL PERMISSIONS FOR select WHERE true;');

  late DataModule data;
  final cache = CacheModule(local, sp, (u) => data.onStreamUpdate(u), logger);
  data = DataModule(
    cache,
    local,
    {
      'thread': {'columns': <String, dynamic>{}}
    },
    logger,
    onHeartbeat: onHeartbeat,
  );
  await data.init('');
  return (
    data,
    () {
      sp.close();
      local.close();
    }
  );
}

void main() {
  final logger = SpookyLogger.root('test');

  test('registering a query starts the TTL heartbeat timer', () async {
    final (data, dispose) = await _buildDataModule(logger);
    addTearDown(dispose);

    final hash = await data.query('thread', 'SELECT * FROM thread', {}, '10m');
    expect(data.getQueryByHash(hash)!.ttlTimer, isNotNull);
  });

  test('heartbeat fires at ~90% of the TTL and reschedules', () async {
    final fired = <String>[];
    final (data, dispose) =
        await _buildDataModule(logger, onHeartbeat: fired.add);
    addTearDown(dispose);

    // ttl '1s' -> heartbeat at 900ms.
    final hash = await data.query('thread', 'SELECT * FROM thread', {}, '1s');
    expect(fired, isEmpty);

    await Future<void>.delayed(const Duration(milliseconds: 1100));
    expect(fired, contains(hash));
    // Timer rescheduled itself, so the lifecycle is still active.
    expect(data.getQueryByHash(hash)!.ttlTimer, isNotNull);
  });
}
