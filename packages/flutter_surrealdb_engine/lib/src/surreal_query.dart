import 'dart:async';
import 'dart:convert';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';

/// A query builder that supports both one-shot execution and live queries.
class SurrealQuery implements Future<String> {
  final SurrealDb _db;
  final String _resource;

  SurrealQuery(this._db, this._resource);

  /// Executes the select query once and returns the JSON result.
  @override
  Future<String> timeout(
    Duration timeLimit, {
    FutureOr<String> Function()? onTimeout,
  }) {
    return _db
        .selectOne(resource: _resource)
        .timeout(timeLimit, onTimeout: onTimeout);
  }

  @override
  Stream<String> asStream() => _db.selectOne(resource: _resource).asStream();

  @override
  Future<String> catchError(
    Function onError, {
    bool Function(Object error)? test,
  }) {
    return _db.selectOne(resource: _resource).catchError(onError, test: test);
  }

  @override
  Future<R> then<R>(
    FutureOr<R> Function(String value) onValue, {
    Function? onError,
  }) {
    return _db.selectOne(resource: _resource).then(onValue, onError: onError);
  }

  @override
  Future<String> whenComplete(FutureOr<void> Function() action) {
    return _db.selectOne(resource: _resource).whenComplete(action);
  }

  /// Establishes a Live Query stream that automatically maintains a list of records.
  ///
  /// The [fromJson] factory is used to convert the raw JSON map into type [T].
  /// The stream works as follows:
  /// 1. Emits the initial snapshot (current state of the table).
  /// 2. Listens for real-time updates (Create, Update, Delete) and updates the local list accordingly.
  /// 3. Emits the updated list on every change.
  Stream<List<T>> live<T>(T Function(Map<String, dynamic> json) fromJson) {
    // We use the internal connectLiveQuery to get the raw event stream
    final stream = _db.liveQuery(tableName: _resource);

    late StreamController<List<T>> controller;
    final Map<String, T> _items = {};

    controller = StreamController<List<T>>(
      onListen: () {
        final subscription = stream.listen(
          (event) {
            try {
              switch (event.action) {
                case LiveQueryAction.snapshot:
                  final List<dynamic> list = jsonDecode(event.result);
                  _items.clear();
                  for (var item in list) {
                    if (item is Map<String, dynamic>) {
                      // Ensure ID exists
                      final id = item['id'] as String?;
                      if (id != null) {
                        _items[id] = fromJson(item);
                      }
                    }
                  }
                  controller.add(_items.values.toList());
                  break;

                case LiveQueryAction.create:
                case LiveQueryAction.update:
                  // Result is the Record Record (JSON Object)
                  final Map<String, dynamic> item = jsonDecode(event.result);
                  final id = item['id'] as String?;
                  if (id != null) {
                    _items[id] = fromJson(item);
                    controller.add(_items.values.toList());
                  }
                  break;

                case LiveQueryAction.delete:
                  // Result might be the ID or the Record.
                  // Rely on event.id if available, or parse id from result.
                  String? id = event.id;
                  if (id == null) {
                    // Try parsing result
                    try {
                      final parsed = jsonDecode(event.result);
                      if (parsed is String)
                        id = parsed;
                      else if (parsed is Map)
                        id = parsed['id'];
                    } catch (_) {}
                  }

                  if (id != null) {
                    _items.remove(id);
                    controller.add(_items.values.toList());
                  }
                  break;

                case LiveQueryAction.unknown:
                  break;
              }
            } catch (e) {
              controller.addError(e);
            }
          },
          onError: controller.addError,
          onDone: controller.close,
        );

        controller.onCancel = subscription.cancel;
      },
    );

    return controller.stream;
  }
}
