import 'dart:ffi';
import 'dart:io';

/// Resolves and opens the `ssp-ffi` native library for the current platform.
///
/// Resolution order:
///  1. The `SSP_FFI_PATH` environment variable, if set (dev / test override).
///  2. A `native/<platform>/` artifact relative to the current directory or
///     the package root (populated by `packages/ssp-ffi/build-native.sh`).
///  3. The plain library name, letting the OS loader search its default paths
///     (covers Flutter bundles where the lib is packaged alongside the app).
DynamicLibrary openSspLibrary() {
  final override = Platform.environment['SSP_FFI_PATH'];
  if (override != null && override.isNotEmpty) {
    return DynamicLibrary.open(override);
  }

  final fileName = _platformFileName();
  final subdir = _platformSubdir();

  for (final base in _searchRoots()) {
    final candidate = '$base/native/$subdir/$fileName';
    if (File(candidate).existsSync()) {
      return DynamicLibrary.open(candidate);
    }
  }

  // Last resort: let the dynamic loader find it by name.
  return DynamicLibrary.open(fileName);
}

String _platformFileName() {
  if (Platform.isMacOS) return 'libssp_ffi.dylib';
  if (Platform.isLinux) return 'libssp_ffi.so';
  if (Platform.isWindows) return 'ssp_ffi.dll';
  throw UnsupportedError(
      'ssp-ffi: unsupported platform ${Platform.operatingSystem}');
}

String _platformSubdir() {
  if (Platform.isMacOS) return 'macos';
  if (Platform.isLinux) return 'linux';
  if (Platform.isWindows) return 'windows';
  throw UnsupportedError(
      'ssp-ffi: unsupported platform ${Platform.operatingSystem}');
}

/// Directories to probe for the bundled native artifact.
List<String> _searchRoots() {
  final roots = <String>['.'];
  // When running tests from the package root, the script dir resolves the
  // package; also probe the directory containing the running script.
  final scriptDir = File.fromUri(Platform.script).parent.path;
  roots.add(scriptDir);
  roots.add('$scriptDir/..');
  return roots;
}
