import 'dart:async';
import 'dart:convert';
import 'dart:math';
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
  String _sanitizeId(String id) {
    // Remove backticks and single quotes that might wrap the ID or parts of it
    return id.replaceAll('`', '').replaceAll("'", "");
  }

  /// Establishes a Live Query stream that automatically maintains a list of records.
  Stream<List<T>> live<T>(T Function(Map<String, dynamic> json) fromJson) {
    // We use the internal connectLiveQuery with snapshot enabled
    final stream = _db.liveQuery(tableName: _resource, snapshot: true);

    late StreamController<List<T>> controller;
    final Map<String, T> items = {};

    controller = StreamController<List<T>>(
      onListen: () {
        final subscription = stream.listen(
          (event) {
            try {
              print(
                "QUERY DEBUG: Action: ${event.action} | Result: ${event.result.substring(0, min(event.result.length, 50))}",
              );

              switch (event.action) {
                case LiveQueryAction.snapshot:
                  print("QUERY: Snapshot received. Parsing...");
                  try {
                    final List<dynamic> list = jsonDecode(event.result);
                    print("QUERY: Snapshot List Size: ${list.length}");
                    items.clear();
                    for (var item in list) {
                      if (item is Map<String, dynamic>) {
                        final rawId = item['id'] as String?;
                        if (rawId != null) {
                          final id = _sanitizeId(rawId);
                          // Update item ID if needed (optional, but good for consistency)
                          item['id'] = id;
                          items[id] = fromJson(item);
                        } else {
                          print(
                            "QUERY WARNING: Item in snapshot has no ID: $item",
                          );
                        }
                      }
                    }
                    controller.add(items.values.toList());
                    print("QUERY: Snapshot processed. Controller updated.");
                  } catch (e) {
                    print("QUERY ERROR (Snapshot): $e");
                  }
                  break;

                case LiveQueryAction.create:
                case LiveQueryAction.update:
                  print("QUERY: Create/Update received.");
                  try {
                    final Map<String, dynamic> item = jsonDecode(event.result);
                    final rawId = item['id'] as String?;
                    if (rawId != null) {
                      final id = _sanitizeId(rawId);
                      print("QUERY: Upserting ID: $id (Raw: $rawId)");
                      item['id'] = id; // Ensure generic map has clean ID
                      items[id] = fromJson(item);
                      controller.add(items.values.toList());
                    } else {
                      print("QUERY WARNING: Create/Update has no ID: $item");
                    }
                  } catch (e) {
                    print("QUERY ERROR (Create/Update): $e");
                  }
                  break;

                case LiveQueryAction.delete:
                  print("QUERY: Delete received. Event ID: ${event.id}");
                  String? id = event.id;
                  if (id == null) {
                    // Fallback parsing
                    try {
                      final parsed = jsonDecode(event.result);
                      if (parsed is String) {
                        id = parsed;
                        print("QUERY: Parsed ID from result string: $id");
                      } else if (parsed is Map) {
                        id = parsed['id'];
                        print("QUERY: Parsed ID from result map: $id");
                      }
                    } catch (_) {}
                  }

                  if (id != null) {
                    id = _sanitizeId(id);
                    print("QUERY: Removing item: $id");
                    if (items.containsKey(id)) {
                      items.remove(id);
                      controller.add(items.values.toList());
                      print("QUERY: Item removed. List size: ${items.length}");
                    } else {
                      print(
                        "QUERY WARNING: Tried to delete $id but it was not in the local list. Known IDs: ${items.keys.join(', ')}",
                      );
                    }
                  } else {
                    print(
                      "QUERY ERROR: Could not determine ID for delete event.",
                    );
                  }
                  break;

                case LiveQueryAction.unknown:
                  print("QUERY: Unknown action received.");
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
