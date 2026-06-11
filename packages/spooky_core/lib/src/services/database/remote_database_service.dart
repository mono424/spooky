import 'dart:async';

import '../../surreal/remote_client.dart';
import '../../types.dart';
import '../../utils/surql.dart';
import '../logger/logger.dart';

/// Remote SurrealDB access over a [RemoteSurrealClient], with the serialized
/// query queue from the JS `AbstractDatabaseService` (prevents overlapping
/// transactions).
class RemoteDatabaseService {
  RemoteDatabaseService(this._config, this._client, SpookyLogger logger)
      : _logger = logger.child('RemoteDatabaseService');

  final DatabaseConfig _config;
  final RemoteSurrealClient _client;
  final SpookyLogger _logger;

  Future<void> _queryQueue = Future<void>.value();

  RemoteSurrealClient getClient() => _client;
  DatabaseConfig getConfig() => _config;

  Future<void> connect() async {
    final endpoint = _config.endpoint;
    if (endpoint == null) {
      _logger.warn('No endpoint configured for remote database');
      return;
    }
    await _client.connect(endpoint);
    await _client.use(namespace: _config.namespace, database: _config.database);
    final token = _config.token;
    if (token != null) {
      await _client.authenticate(token);
    }
    _logger.info('Connected to remote database');
  }

  /// Serialized query: chains onto [_queryQueue] so calls never overlap.
  Future<List<dynamic>> query(String sql, [Map<String, dynamic>? vars]) {
    final completer = Completer<List<dynamic>>();
    _queryQueue = _queryQueue.then((_) async {
      try {
        completer.complete(await _client.query(sql, vars));
      } catch (err) {
        completer.completeError(err);
      }
    }).catchError((Object err) {
      // The query's own error already reached the caller via `completeError`
      // above; swallow here only so one failed link can't wedge the serialized
      // queue for every subsequent query.
      _logger.debug('Serialized query link failed (already surfaced): $err');
    });
    return completer.future;
  }

  Future<T> execute<T>(SealedQuery<T> query,
      [Map<String, dynamic>? vars]) async {
    final raw = await this.query(query.sql, vars);
    return query.extract(raw);
  }

  Future<dynamic> signin(Map<String, dynamic> params) => _client.signin(params);
  Future<dynamic> signup(Map<String, dynamic> params) => _client.signup(params);
  Future<dynamic> authenticate(String token) => _client.authenticate(token);
  Future<void> invalidate() => _client.invalidate();

  Future<void> close() => _client.close();
}
