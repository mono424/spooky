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

/// SurrealQL select for a `.related()` query's SUBQUERY child edges
/// (TS `buildSubqueryListRefSelect`). The SSP tags each matched child edge with
/// `parent`/`parent_rel`, at any nesting depth, so `parent IS NOT NONE` is
/// exactly the complement of [buildListRefSelect]'s primary window.
String buildSubqueryListRefSelect(String table) =>
    'SELECT out, version FROM $table WHERE in = \$in AND parent IS NOT NONE';

/// Resolve the effective poll interval; non-positive falls back to the default.
int resolveListRefPollInterval(int? opt) {
  if (opt == null || opt <= 0) return defaultListRefPollIntervalMs;
  return opt;
}

/// Ceiling for the adaptive `_00_list_ref` poll backoff (ms)
/// (TS `LIST_REF_POLL_MAX_INTERVAL_MS`). An idle client coasts up to this
/// cadence, so the worst-case catch-up latency for a missed cross-session
/// change stays at the 5s the codebase already treats as acceptable.
const int listRefPollMaxIntervalMs = 5000;

/// Adaptive poll delay (TS `listRefPollDelayMs`): stay at the responsive
/// [baseIntervalMs] while changes are arriving, and exponentially back off
/// toward [maxIntervalMs] while `_00_list_ref` is quiet.
///
/// [idleStreak] is the count of consecutive poll cycles that observed no
/// change. [Sp00kySync] resets it to 0 whenever a poll detects a real
/// remoteArray change OR a LIVE event lands, so any activity snaps the poll
/// straight back to [baseIntervalMs].
///
/// This supersedes [nextPollDelayMs], which slowed the poll only while LIVE was
/// delivering; the cross-session LIVE-permission gap means LIVE frequently never
/// fires, which left a fully idle client polling every base interval forever.
/// Backing off on observed idleness covers the LIVE-healthy case for free (LIVE
/// applies the change, the next poll sees nothing new, the streak grows).
int listRefPollDelayMs({
  required int idleStreak,
  required int baseIntervalMs,
  int maxIntervalMs = listRefPollMaxIntervalMs,
}) {
  final cap =
      maxIntervalMs > baseIntervalMs ? maxIntervalMs : baseIntervalMs;
  if (idleStreak <= 0) return baseIntervalMs;
  // Clamp the exponent so a long-idle client can't overflow.
  final exponent = idleStreak > 30 ? 30 : idleStreak;
  final delay = baseIntervalMs * (1 << exponent);
  return delay < cap ? delay : cap;
}

/// Order-insensitive equality for two [RecordVersionArray]s
/// (TS `recordVersionArraysEqual`). The `_00_list_ref` SELECT has no
/// `ORDER BY`, so row order can differ between polls without anything having
/// changed; comparing as an id -> version map avoids false "changed" verdicts
/// that would defeat the idle backoff. Record ids are unique within a query's
/// list_ref, so a map is a faithful representation.
bool recordVersionArraysEqual(RecordVersionArray a, RecordVersionArray b) {
  if (a.length != b.length) return false;
  final byId = {for (final e in a) e.$1: e.$2};
  for (final e in b) {
    if (byId[e.$1] != e.$2) return false;
  }
  return true;
}

/// Pick the next poll delay based on LIVE health (TS `nextPollDelayMs`). Pure
/// for unit-testing.
///
/// Superseded by [listRefPollDelayMs], which backs off on observed change
/// activity (LIVE *or* poll-detected) rather than LIVE liveness alone. Kept
/// (and tested) for reference, matching the TS core.
@Deprecated('Use listRefPollDelayMs')
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
