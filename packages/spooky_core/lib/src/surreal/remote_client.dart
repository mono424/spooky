import 'dart:async';
import 'dart:convert';

import 'package:meta/meta.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

import 'value.dart';

/// A LIVE-query notification (TS SurrealDB `live` message).
class LiveMessage {
  const LiveMessage(this.action, this.value);

  /// `'CREATE' | 'UPDATE' | 'DELETE' | 'KILLED'`.
  final String action;
  final Map<String, dynamic> value;
}

/// The subset of the SurrealDB client the core needs. Decouples the core from
/// the transport so a community package could be dropped in later.
abstract class RemoteSurrealClient {
  Future<void> connect(String endpoint);
  Future<void> use({required String namespace, required String database});
  Future<dynamic> authenticate(String token);
  Future<dynamic> signin(Map<String, dynamic> params);
  Future<dynamic> signup(Map<String, dynamic> params);
  Future<void> invalidate();

  /// Run a SURQL query; returns the per-statement results array.
  Future<List<dynamic>> query(String sql, [Map<String, dynamic>? vars]);

  /// Start a LIVE query; returns the live id and a stream of notifications.
  Future<(String liveId, Stream<LiveMessage>)> live(String sql,
      [Map<String, dynamic>? vars]);

  Future<void> kill(String liveId);

  /// Connection lifecycle: emits on connect / disconnect.
  Stream<void> get onConnected;
  Stream<void> get onDisconnected;

  Future<void> close();
}

/// Minimal SurrealDB WebSocket JSON-RPC client.
///
/// Implements only what the core uses (`use`/`signin`/`signup`/`authenticate`/
/// `invalidate`/`query`/`live`/`kill` + lifecycle). JSON wire format keeps
/// record-id/datetime decoding explicit.
///
/// NOTE: this path requires a running SurrealDB/ssp server to verify
/// end-to-end; it is the integration risk called out in the plan.
class WebSocketSurrealClient implements RemoteSurrealClient {
  WebSocketSurrealClient();

  WebSocketChannel? _channel;
  int _nextId = 0;
  final Map<String, Completer<dynamic>> _pending = {};
  final Map<String, StreamController<LiveMessage>> _liveControllers = {};
  final StreamController<void> _connected = StreamController.broadcast();
  final StreamController<void> _disconnected = StreamController.broadcast();

  @override
  Stream<void> get onConnected => _connected.stream;
  @override
  Stream<void> get onDisconnected => _disconnected.stream;

  /// Opens the WebSocket for [uri]. Overridable in tests to inject a fake
  /// channel (see `@visibleForTesting`).
  @visibleForTesting
  WebSocketChannel createChannel(Uri uri) => WebSocketChannel.connect(uri);

  /// The RPC endpoint a raw [endpoint] normalizes to (exposed for tests).
  @visibleForTesting
  String rpcEndpoint(String endpoint) => _rpcEndpoint(endpoint);

  @override
  Future<void> connect(String endpoint) async {
    final uri = Uri.parse(_rpcEndpoint(endpoint));
    final channel = createChannel(uri);
    _channel = channel;
    await channel.ready;
    channel.stream.listen(
      _onMessage,
      onDone: () => _disconnected.add(null),
      onError: (_) => _disconnected.add(null),
      cancelOnError: false,
    );
    _connected.add(null);
  }

  String _rpcEndpoint(String endpoint) {
    var e = endpoint;
    if (e.startsWith('http://')) e = 'ws://${e.substring(7)}';
    if (e.startsWith('https://')) e = 'wss://${e.substring(8)}';
    if (!e.endsWith('/rpc')) e = '${e.replaceAll(RegExp(r'/$'), '')}/rpc';
    return e;
  }

  Future<dynamic> _rpc(String method, List<dynamic> params) {
    final channel = _channel;
    if (channel == null) {
      throw StateError('WebSocket not connected');
    }
    final id = (_nextId++).toString();
    final completer = Completer<dynamic>();
    _pending[id] = completer;
    channel.sink
        .add(jsonEncode({'id': id, 'method': method, 'params': params}));
    return completer.future;
  }

  void _onMessage(dynamic raw) {
    final decoded = jsonDecode(raw as String);
    if (decoded is! Map) return;
    final map = decoded.cast<String, dynamic>();

    // LIVE notifications carry no matching request id.
    final result = map['result'];
    if (map['id'] == null && result is Map && result['id'] != null) {
      _dispatchLive(result.cast<String, dynamic>());
      return;
    }

    final id = map['id']?.toString();
    if (id == null) return;
    final completer = _pending.remove(id);
    if (completer == null) return;
    if (map['error'] != null) {
      final err = map['error'];
      completer.completeError(StateError(err is Map
          ? (err['message']?.toString() ?? err.toString())
          : err.toString()));
    } else {
      completer.complete(map['result']);
    }
  }

  void _dispatchLive(Map<String, dynamic> notification) {
    final liveId = notification['id'].toString();
    final action = (notification['action'] as String?)?.toUpperCase() ?? '';
    final value =
        (notification['result'] as Map?)?.cast<String, dynamic>() ?? {};
    final controller = _liveControllers[liveId];
    controller?.add(LiveMessage(action, value));
  }

  String? _ns;
  String? _db;

  @override
  Future<void> use({required String namespace, required String database}) {
    _ns = namespace;
    _db = database;
    return _rpc('use', [namespace, database]);
  }

  @override
  Future<dynamic> authenticate(String token) => _rpc('authenticate', [token]);

  @override
  Future<dynamic> signin(Map<String, dynamic> params) =>
      _rpc('signin', [_authParams(params)]);

  @override
  Future<dynamic> signup(Map<String, dynamic> params) =>
      _rpc('signup', [_authParams(params)]);

  /// Normalize auth params to the SurrealDB RPC wire shape.
  ///
  /// `AuthService` (faithful to the JS SDK) passes `{access, variables}`; the
  /// raw RPC instead wants `{ns, db, ac, ...variables}` with the access
  /// variables flattened. Root logins (`{user, pass}`) pass through unchanged.
  Map<String, dynamic> _authParams(Map<String, dynamic> params) {
    if (!params.containsKey('access') && !params.containsKey('variables')) {
      return params; // e.g. root {user, pass}
    }
    final vars =
        (params['variables'] as Map?)?.cast<String, dynamic>() ?? const {};
    return {
      if (_ns != null) 'ns': _ns,
      if (_db != null) 'db': _db,
      if (params['access'] != null) 'ac': params['access'],
      ...vars,
    };
  }

  @override
  Future<void> invalidate() => _rpc('invalidate', []);

  @override
  Future<List<dynamic>> query(String sql, [Map<String, dynamic>? vars]) async {
    final result = await _rpc('query', [sql, _encodeVars(vars)]);
    // SurrealDB returns [{ status, result }, ...]; surface the result list to
    // match the JS `.query()` which returns the per-statement results.
    //
    // A statement with status 'ERR' throws (matching the JS SDK). This is
    // load-bearing for sync: a `LIVE SELECT` on a not-yet-created
    // `_00_list_ref_user_<id>` table fails per-statement in v3, and
    // Sp00kySync's retry backoff relies on the throw to re-attempt.
    if (result is List) {
      return result.map((stmt) {
        if (stmt is Map && stmt.containsKey('result')) {
          if (stmt['status'] != null && stmt['status'] != 'OK') {
            throw StateError('Query error: ${stmt['result']}');
          }
          return stmt['result'];
        }
        return stmt;
      }).toList();
    }
    return [result];
  }

  @override
  Future<(String, Stream<LiveMessage>)> live(String sql,
      [Map<String, dynamic>? vars]) async {
    final results = await query(sql, vars);
    final liveId = results.isNotEmpty ? results.first.toString() : '';
    final controller = StreamController<LiveMessage>.broadcast();
    _liveControllers[liveId] = controller;
    return (liveId, controller.stream);
  }

  @override
  Future<void> kill(String liveId) async {
    await _rpc('kill', [liveId]);
    await _liveControllers.remove(liveId)?.close();
  }

  /// Encode bind vars to JSON-safe values ([RecordId] -> `table:id`, etc.).
  Map<String, dynamic> _encodeVars(Map<String, dynamic>? vars) {
    if (vars == null) return {};
    dynamic enc(dynamic v) {
      if (v is RecordId) return v.encode();
      if (v is DateTime) return v.toUtc().toIso8601String();
      if (v is Map) return v.map((k, val) => MapEntry(k.toString(), enc(val)));
      if (v is List) return v.map(enc).toList();
      return v;
    }

    return vars.map((k, v) => MapEntry(k, enc(v)));
  }

  @override
  Future<void> close() async {
    await _channel?.sink.close();
    for (final c in _liveControllers.values) {
      await c.close();
    }
    _liveControllers.clear();
    await _connected.close();
    await _disconnected.close();
  }
}
