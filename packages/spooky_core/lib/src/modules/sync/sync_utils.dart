import '../../surreal/value.dart';
import '../../types.dart';
import '../../utils/record_id_utils.dart';

/// Default cadence for the `_00_list_ref` poll fallback (ms).
const int defaultListRefPollIntervalMs = 500;

/// LIVE-healthy cooldown window (ms): a LIVE event within this window backs the
/// poll off to [liveHealthyPollIntervalMs].
const int liveHealthyCooldownMs = 5000;

/// Poll interval while LIVE is delivering events (ms).
const int liveHealthyPollIntervalMs = 5000;

/// Incrementally maintains a local version array and diffs it against the
/// remote array (TS `ArraySyncer`).
class ArraySyncer {
  ArraySyncer(RecordVersionArray localArray, RecordVersionArray remoteArray)
      : _remoteArray = [...remoteArray]..sort((a, b) => a.$1.compareTo(b.$1)),
        _localArray = [...localArray]..sort((a, b) => a.$1.compareTo(b.$1));

  RecordVersionArray _localArray;
  final RecordVersionArray _remoteArray;
  bool _needsSort = false;

  void insert(String recordId, int version) {
    _localArray.add((recordId, version));
    _needsSort = true;
  }

  void update(String recordId, int version) {
    _localArray = _localArray.map((record) {
      if (record.$1 == recordId) {
        _needsSort = true;
        return (recordId, version);
      }
      return record;
    }).toList();
  }

  void delete(String recordId) {
    _localArray = _localArray.where((record) => record.$1 != recordId).toList();
  }

  RecordVersionDiff? nextSet() {
    if (_needsSort) {
      _localArray.sort((a, b) => a.$1.compareTo(b.$1));
      _needsSort = false;
    }
    return diffRecordVersionArray(_localArray, _remoteArray);
  }
}

/// Diff two version arrays into added/updated/removed (TS `diffRecordVersionArray`).
RecordVersionDiff diffRecordVersionArray(
  RecordVersionArray? local,
  RecordVersionArray? remote,
) {
  final localMap = {for (final e in local ?? const []) e.$1: e.$2};
  final remoteMap = {for (final e in remote ?? const []) e.$1: e.$2};

  final added = <String>[];
  final updated = <String>[];
  final removed = <String>[];

  remoteMap.forEach((recordId, remoteVersion) {
    final localVersion = localMap[recordId];
    if (localVersion == null) {
      added.add(recordId);
    } else if (localVersion < remoteVersion) {
      updated.add(recordId);
    }
  });

  for (final recordId in localMap.keys) {
    if (!remoteMap.containsKey(recordId)) removed.add(recordId);
  }

  return RecordVersionDiff(
    added: added
        .map<({RecordId id, int version})>(
            (id) => (id: parseRecordIdString(id), version: remoteMap[id]!))
        .toList(),
    updated: updated
        .map<({RecordId id, int version})>(
            (id) => (id: parseRecordIdString(id), version: remoteMap[id]!))
        .toList(),
    removed: removed.map(parseRecordIdString).toList(),
  );
}

/// Build a one-record-op diff from a LIVE `_00_list_ref` change
/// (TS `createDiffFromDbOp`). Returns an empty diff if the cached version is
/// already at least [version].
RecordVersionDiff createDiffFromDbOp(
  String op,
  RecordId recordId,
  int version, [
  RecordVersionArray? versions,
]) {
  final encoded = encodeRecordId(recordId);
  final old = versions?.where((r) => r.$1 == encoded).firstOrNull;
  if (old != null && old.$2 >= version) {
    return RecordVersionDiff(added: [], updated: [], removed: []);
  }
  switch (op) {
    case 'CREATE':
      return RecordVersionDiff(
          added: [(id: recordId, version: version)], updated: [], removed: []);
    case 'UPDATE':
      return RecordVersionDiff(
          added: [], updated: [(id: recordId, version: version)], removed: []);
    default:
      return RecordVersionDiff(added: [], updated: [], removed: [recordId]);
  }
}

/// SurrealQL select powering the initial fetch and the poll of
/// `_00_list_ref[_user_<id>]` (TS `buildListRefSelect`).
String buildListRefSelect(String table) =>
    'SELECT out, version FROM $table WHERE in = \$in AND parent IS NONE';

/// Resolve the effective poll interval; non-positive falls back to the default.
int resolveListRefPollInterval(int? opt) {
  if (opt == null || opt <= 0) return defaultListRefPollIntervalMs;
  return opt;
}

/// Pick the next poll delay based on LIVE health (TS `nextPollDelayMs`). Pure
/// for unit-testing.
int nextPollDelayMs({
  required int now,
  required int? lastLiveEventAt,
  required int baseIntervalMs,
  int cooldownMs = liveHealthyCooldownMs,
  int healthyIntervalMs = liveHealthyPollIntervalMs,
}) {
  if (lastLiveEventAt == null) return baseIntervalMs;
  final sinceLive = now - lastLiveEventAt;
  if (sinceLive < 0 || sinceLive >= cooldownMs) return baseIntervalMs;
  return healthyIntervalMs > baseIntervalMs
      ? healthyIntervalMs
      : baseIntervalMs;
}

extension _FirstOrNull<E> on Iterable<E> {
  E? get firstOrNull {
    final it = iterator;
    return it.moveNext() ? it.current : null;
  }
}
