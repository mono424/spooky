import '../../types.dart';

/// Simple in-memory [PersistenceClient]. Used as the default backing store for
/// stream-processor state and the auth token until a sqlite-backed client is
/// wired in. State does not survive process restarts.
class MemoryPersistenceClient implements PersistenceClient {
  final Map<String, dynamic> _store = {};

  @override
  Future<void> set(String key, dynamic value) async {
    _store[key] = value;
  }

  @override
  Future<T?> get<T>(String key) async => _store[key] as T?;

  @override
  Future<void> remove(String key) async {
    _store.remove(key);
  }
}
