import 'dart:convert';
import '../types.dart';
import 'db.dart';

class QueryManager {
  final DatabaseService db;
  final Map<int, Set<Function(dynamic)>> subscriptions = {};
  final Map<int, Incantation> activeQueries = {};

  QueryManager(this.db);

  Future<int> register(String surrealql) async {
    final queryHash = _hashString(surrealql);

    if (activeQueries.containsKey(queryHash)) {
      return queryHash;
    }

    final incantation = Incantation(
      id: queryHash,
      surrealql: surrealql,
      hash: 0,
      lastActiveAt: DateTime.now().millisecondsSinceEpoch,
    );

    activeQueries[queryHash] = incantation;

    await _initLifecycle(incantation);

    return queryHash;
  }

  Function() subscribe(int queryHash, Function(dynamic) callback) {
    if (!subscriptions.containsKey(queryHash)) {
      subscriptions[queryHash] = {};
    }
    subscriptions[queryHash]!.add(callback);

    // Refresh local
    _refreshLocal(queryHash).then((data) {
      callback(data);
    });

    return () {
      final subs = subscriptions[queryHash];
      if (subs != null) {
        subs.remove(callback);
        if (subs.isEmpty) {
          subscriptions.remove(queryHash);
          // Optional: Stop lifecycle if no subscribers
        }
      }
    };
  }

  Future<void> _initLifecycle(Incantation incantation) async {
    // 1. Local Hydration
    await _refreshLocal(incantation.id);

    // 2. Remote Registration & Sync
    await _registerRemote(incantation);

    // 3. Start Live Query
    await _startLiveQuery(incantation);
  }

  Future<dynamic> _refreshLocal(int queryHash) async {
    final incantation = activeQueries[queryHash];
    if (incantation == null) return null;

    try {
      final results = await db.queryLocal<dynamic>(incantation.surrealql);

      // Calculate Hash
      final currentHash = _calculateHash(results);

      if (currentHash != incantation.hash) {
        incantation.hash = currentHash;
        _notifySubscribers(queryHash, results);
      }

      return results;
    } catch (e) {
      print("Error refreshing local query ${incantation.id}: $e");
      return null;
    }
  }

  Future<void> _registerRemote(Incantation incantation) async {
    // Check if incantation exists remotely
    try {
      final remoteIncantations = await db.queryRemote<List<dynamic>>(
        r'SELECT * FROM spooky_incantation WHERE id = $id',
        {'id': incantation.id},
      );

      if (remoteIncantations.isEmpty) {
        await db.queryRemote(r'CREATE spooky_incantation CONTENT $data', {
          'data': incantation.toJson(),
        });
      }
    } catch (e) {
      print("Error registering remote incantation: $e");
    }
  }

  Future<void> _startLiveQuery(Incantation incantation) async {
    // Placeholder: Listen to changes on the spooky_incantation table for this specific ID
    // or listen to the actual query if supported.
    // The README says: "First, we set up a LIVE QUERY that listens to that remote Incantation"
    await db.subscribeLive('spooky_incantation', (action, result) {
      // Check if this update relates to our incantation
      // This is a simplified check
      if (result['id'] == incantation.id) {
        // Trigger refresh or sync
        _refreshLocal(incantation.id);
      }
    });
  }

  int _calculateHash(dynamic data) {
    final str = jsonEncode(data);
    return _hashString(str);
  }

  int _hashString(String str) {
    int hash = 0;
    for (int i = 0; i < str.length; i++) {
      int char = str.codeUnitAt(i);
      hash = (hash << 5) - hash + char;
      // Simulate 32-bit integer overflow
      hash &= 0xFFFFFFFF;
      if (hash > 0x7FFFFFFF) hash -= 0x100000000;
    }
    return hash;
  }

  void _notifySubscribers(int queryHash, dynamic data) {
    final subs = subscriptions[queryHash];
    if (subs != null) {
      for (final cb in subs) {
        cb(data);
      }
    }
  }
}
