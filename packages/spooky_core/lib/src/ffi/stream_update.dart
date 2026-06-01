/// A `(recordId, version)` pair, mirroring the WASM `[string, number]` tuple.
typedef RecordVersion = (String, int);

/// In-memory representation of a query result set as id/version pairs.
///
/// Mirrors the TS `RecordVersionArray = Array<[string, number]>`.
typedef RecordVersionArray = List<RecordVersion>;

/// A single record's id + version (matches `WasmDeltaRecord`).
class DeltaRecord {
  const DeltaRecord(this.id, this.version);

  final String id;
  final int version;

  factory DeltaRecord.fromJson(List<dynamic> json) =>
      DeltaRecord(json[0] as String, (json[1] as num).toInt());
}

/// Granular delta describing what changed (matches `WasmDelta`).
class ViewDelta {
  const ViewDelta({
    required this.additions,
    required this.removals,
    required this.updates,
  });

  final List<DeltaRecord> additions;
  final List<String> removals;
  final List<DeltaRecord> updates;

  factory ViewDelta.fromJson(Map<String, dynamic> json) => ViewDelta(
        additions: (json['additions'] as List<dynamic>)
            .map((e) => DeltaRecord.fromJson(e as List<dynamic>))
            .toList(),
        removals: (json['removals'] as List<dynamic>).cast<String>(),
        updates: (json['updates'] as List<dynamic>)
            .map((e) => DeltaRecord.fromJson(e as List<dynamic>))
            .toList(),
      );

  static const empty = ViewDelta(additions: [], removals: [], updates: []);
}

/// A materialized-view update emitted by the stream processor.
///
/// Mirrors the TS `StreamUpdate` (the local store path consumes [localArray];
/// [delta] and [resultHash] are carried for receivers that want them).
class StreamUpdate {
  const StreamUpdate({
    required this.queryHash,
    required this.localArray,
    this.resultHash = '',
    this.delta = ViewDelta.empty,
    this.op,
    this.materializationTimeMs,
  });

  /// The query id this update is for (WASM `query_id`).
  final String queryHash;

  /// The full result set as id/version pairs (WASM `result_data`).
  final RecordVersionArray localArray;

  final String resultHash;
  final ViewDelta delta;

  /// Operation that produced this update; null for the register snapshot.
  final String? op;

  /// End-to-end ingest latency for the FFI call that produced this update.
  final double? materializationTimeMs;

  /// Build from the decoded `WasmViewUpdate` JSON.
  factory StreamUpdate.fromWasm(
    Map<String, dynamic> json, {
    String? op,
    double? materializationTimeMs,
  }) {
    final resultData = (json['result_data'] as List<dynamic>)
        .map<RecordVersion>(
            (e) => ((e as List<dynamic>)[0] as String, (e[1] as num).toInt()))
        .toList();
    return StreamUpdate(
      queryHash: json['query_id'] as String,
      localArray: resultData,
      resultHash: (json['result_hash'] as String?) ?? '',
      delta: json['delta'] != null
          ? ViewDelta.fromJson(json['delta'] as Map<String, dynamic>)
          : ViewDelta.empty,
      op: op,
      materializationTimeMs: materializationTimeMs,
    );
  }
}

/// Thrown when the native processor returns an `{"err": ...}` envelope.
class SspException implements Exception {
  SspException(this.message);
  final String message;
  @override
  String toString() => 'SspException: $message';
}
