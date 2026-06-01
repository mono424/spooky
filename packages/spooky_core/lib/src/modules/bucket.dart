import '../services/database/remote_database_service.dart';

/// A handle to a SurrealDB storage bucket (TS `BucketHandle`). Operations are
/// issued as `f"<bucket>:/<path>"....` file expressions over the remote.
///
/// Return-value parsing is tolerant (null-safe) so a backend that returns no
/// body for an op doesn't throw.
class BucketHandle {
  BucketHandle(this._bucketName, this._remote);

  final String _bucketName;
  final RemoteDatabaseService _remote;

  String _ref(String path) => 'f"$_bucketName:/$path"';

  Future<void> put(String path, Object content) async {
    await _remote
        .query('RETURN ${_ref(path)}.put(\$content);', {'content': content});
  }

  Future<dynamic> get(String path) async {
    final result = await _remote.query('RETURN ${_ref(path)}.get();');
    return result.isNotEmpty ? result.first : null;
  }

  Future<void> delete(String path) async {
    await _remote.query('RETURN ${_ref(path)}.delete();');
  }

  Future<bool> exists(String path) async {
    final result = await _remote.query('RETURN ${_ref(path)}.exists();');
    return result.isNotEmpty && result.first == true;
  }

  Future<Map<String, dynamic>> head(String path) async {
    final result = await _remote.query('RETURN ${_ref(path)}.head();');
    final first = result.isNotEmpty ? result.first : null;
    return (first as Map?)?.cast<String, dynamic>() ?? {};
  }

  Future<void> copy(String sourcePath, String targetPath) async {
    await _remote.query(
        'RETURN ${_ref(sourcePath)}.copy(\$target);', {'target': targetPath});
  }

  Future<void> rename(String sourcePath, String targetPath) async {
    await _remote.query(
        'RETURN ${_ref(sourcePath)}.rename(\$target);', {'target': targetPath});
  }

  Future<List<String>> list([String? prefix]) async {
    final p = prefix ?? '';
    final result = await _remote.query('RETURN ${_ref(p)}.list();');
    final first = result.isNotEmpty ? result.first : null;
    return (first as List?)?.cast<String>() ?? [];
  }
}
