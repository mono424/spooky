import 'surrealdb_platform_interface.dart';
import 'dart:async';
import 'src/rust/api/client.dart' as generated;
import 'src/rust/api/live_query/models.dart';

export 'src/rust/api/client.dart' show StorageMode;
export 'src/rust/frb_generated.dart' show RustLib;
export 'src/rust/api/live_query/models.dart';
export 'src/surreal_query.dart';
import 'src/surreal_query.dart';

/// Wrapper around the generated SurrealDb to provide enhanced functionality
/// specifically for Live Query cancellations.
class SurrealDb {
  final generated.SurrealDb _inner;

  SurrealDb._(this._inner);

  static Future<SurrealDb> connect({
    required generated.StorageMode mode,
  }) async {
    final inner = await generated.SurrealDb.connect(mode: mode);
    return SurrealDb._(inner);
  }

  Future<void> authenticate({required String token}) =>
      _inner.authenticate(token: token);

  Future<void> close() => _inner.close();

  Future<String> create({required String resource, String? data}) =>
      _inner.create(resource: resource, data: data);

  Future<String> delete({required String resource}) =>
      _inner.delete(resource: resource);

  Future<void> export_({required String path}) => _inner.export_(path: path);

  Future<void> invalidate() => _inner.invalidate();

  static Future<void> killQuery({required String queryUuid}) =>
      generated.SurrealDb.killQuery(queryUuid: queryUuid);

  Stream<LiveQueryEvent> liveQuery({required String tableName}) {
    // Use the new connectLiveQuery which guarantees Snapshot + Stream
    final stream = _inner.connectLiveQuery(table: tableName);
    String? capturedUuid;

    // Create a controller to intercept the stream
    final controller = StreamController<LiveQueryEvent>();

    controller.onListen = () {
      final subscription = stream.listen(
        (event) {
          // Capture UUID from Snapshot event or any event carrying it
          if (event.queryUuid != null && capturedUuid == null) {
            capturedUuid = event.queryUuid;
          }
          controller.add(event);
        },
        onError: controller.addError,
        onDone: controller.close,
      );

      controller.onCancel = () async {
        if (capturedUuid != null) {
          print("Auto-Killing Query: $capturedUuid");
          // Fire and forget (or with short timeout) to avoid blocking UI if backend is locked
          // We don't await this strictly for the subscription cancel to proceed
          generated.SurrealDb.killQuery(
            queryUuid: capturedUuid!,
          ).timeout(const Duration(milliseconds: 500)).catchError((e) {
            print("Error killing query (timeout/ignore): $e");
          });
        }
        await subscription.cancel();
      };
    };

    return controller.stream;
  }

  Future<String> merge({required String resource, String? data}) =>
      _inner.merge(resource: resource, data: data);

  Future<String> query({required String sql, String? vars}) =>
      _inner.query(sql: sql, vars: vars);

  Future<void> queryBegin() => _inner.queryBegin();

  Future<void> queryCancel() => _inner.queryCancel();

  Future<void> queryCommit() => _inner.queryCommit();

  /// Selects all records from a table or a specific record.
  /// Returns a [SurrealQuery] which can be awaited to get the result once,
  /// or used to start a [live] query.
  SurrealQuery select({required String resource}) =>
      SurrealQuery(this, resource);

  /// Helper method for one-shot select (used by SurrealQuery)
  Future<String> selectOne({required String resource}) =>
      _inner.select(resource: resource);

  Future<String> signin({required String creds}) => _inner.signin(creds: creds);

  Future<String> signup({required String creds}) => _inner.signup(creds: creds);

  Future<String> transaction({required String stmts, String? vars}) =>
      _inner.transaction(stmts: stmts, vars: vars);

  Future<String> update({required String resource, String? data}) =>
      _inner.update(resource: resource, data: data);

  Future<void> useDb({required String ns, required String db}) =>
      _inner.useDb(ns: ns, db: db);

  Future<String?> getPlatformVersion() {
    return SurrealdbPlatform.instance.getPlatformVersion();
  }
}
