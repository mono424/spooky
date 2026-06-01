import 'dart:async';

import 'events/event_system.dart';
import 'ffi/stream_update.dart' show RecordVersionArray;
import 'surreal/value.dart';
import 'utils/duration_utils.dart';

export 'ffi/stream_update.dart' show RecordVersion, RecordVersionArray;
export 'utils/duration_utils.dart' show QueryTimeToLive, defaultTtl;

/// Local store backend (TS `StoreType`).
enum StoreType { memory, indexeddb }

/// Custom storage backend interface (TS `PersistenceClient`).
abstract class PersistenceClient {
  Future<void> set(String key, dynamic value);
  Future<T?> get<T>(String key);
  Future<void> remove(String key);
}

/// Result of registering/executing a query (TS `Sp00kyQueryResult`).
class Sp00kyQueryResult {
  const Sp00kyQueryResult(this.hash);
  final String hash;
}

/// Database connection configuration (TS `Sp00kyConfig.database`).
class DatabaseConfig {
  const DatabaseConfig({
    this.endpoint,
    required this.namespace,
    required this.database,
    this.store = StoreType.memory,
    this.token,
  });

  final String? endpoint;
  final String namespace;
  final String database;
  final StoreType store;
  final String? token;
}

/// Configuration for the Sp00ky client (TS `Sp00kyConfig`).
class Sp00kyConfig {
  const Sp00kyConfig({
    required this.database,
    required this.schema,
    required this.schemaSurql,
    this.logLevel = 'info',
    this.persistenceClient,
    this.streamDebounceTime = 100,
    this.crdtDebounceMs = 500,
    this.refSyncIntervalMs = 500,
  });

  final DatabaseConfig database;

  /// The schema definition: table name -> column name -> [ColumnSchema].
  /// (The TS generic `S extends SchemaStructure` collapses to runtime data.)
  final Map<String, dynamic> schema;

  /// The compiled SURQL schema string (used for provisioning + permissions).
  final String schemaSurql;
  final String logLevel;
  final dynamic persistenceClient;
  final int streamDebounceTime;
  final int crdtDebounceMs;
  final int refSyncIntervalMs;
}

typedef QueryHash = String;

/// Difference between two record-version sets (TS `RecordVersionDiff`).
class RecordVersionDiff {
  RecordVersionDiff({
    required this.added,
    required this.updated,
    required this.removed,
  });

  final List<({RecordId id, int version})> added;
  final List<({RecordId id, int version})> updated;
  final List<RecordId> removed;
}

/// Configuration for a specific query instance (TS `QueryConfig`).
///
/// Fields are mutable: `processStreamUpdate` and sync mutate them in place.
class QueryConfig {
  QueryConfig({
    required this.id,
    required this.surql,
    required this.params,
    required this.localArray,
    required this.remoteArray,
    required this.ttl,
    required this.lastActiveAt,
    required this.tableName,
  });

  final RecordId id;
  String surql;
  Map<String, dynamic> params;
  RecordVersionArray localArray;
  RecordVersionArray remoteArray;
  QueryTimeToLive ttl;
  DateTime lastActiveAt;
  String tableName;
}

/// Cap on the rolling materialization-sample window per query
/// (TS `MATERIALIZATION_SAMPLE_WINDOW`).
const int materializationSampleWindow = 100;

/// Internal state of a live query (TS `QueryState`).
class QueryState {
  QueryState({
    required this.config,
    List<Map<String, dynamic>>? records,
    this.ttlTimer,
    this.ttlDurationMs = 0,
    this.updateCount = 0,
    List<double>? materializationSamples,
    this.lastIngestLatencyMs,
    this.errorCount = 0,
  })  : records = records ?? [],
        materializationSamples = materializationSamples ?? [];

  QueryConfig config;
  List<Map<String, dynamic>> records;
  Timer? ttlTimer;
  int ttlDurationMs;
  int updateCount;
  List<double> materializationSamples;
  double? lastIngestLatencyMs;
  int errorCount;
}

/// Notified with the latest result set for a query (TS `QueryUpdateCallback`).
typedef QueryUpdateCallback = void Function(List<Map<String, dynamic>> records);

/// Mutation kind (TS `MutationEventType`).
enum MutationEventType { create, update, delete }

/// A mutation to be synchronized (TS `MutationEvent`).
class MutationEvent {
  MutationEvent({
    required this.type,
    required this.mutationId,
    required this.recordId,
    this.data,
    this.record,
    this.options,
    required this.createdAt,
  });

  final MutationEventType type;
  final RecordId mutationId;
  final RecordId recordId;
  final dynamic data;
  final dynamic record;
  final PushEventOptions? options;
  final DateTime createdAt;
}

/// Options for `run` operations (TS `RunOptions`).
class RunOptions {
  const RunOptions({
    this.assignedTo,
    this.maxRetries,
    this.retryStrategy,
    this.timeout,
  });

  final String? assignedTo;
  final int? maxRetries;
  final String? retryStrategy;
  final int? timeout;
}

/// Debounce key strategy for updates (TS `DebounceOptions.key`).
enum DebounceKey { recordId, recordIdXFields }

/// Configuration for debouncing updates (TS `DebounceOptions`).
class DebounceOptions {
  const DebounceOptions({this.key, this.delay});
  final DebounceKey? key;
  final int? delay;
}

/// Options for update operations (TS `UpdateOptions`).
///
/// `debounced` is either a bool (default behavior) or a [DebounceOptions].
class UpdateOptions {
  const UpdateOptions({this.debounced});
  final Object? debounced;
}
