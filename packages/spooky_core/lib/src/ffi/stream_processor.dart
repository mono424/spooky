import 'dart:convert';
import 'dart:ffi';

import 'package:ffi/ffi.dart';

import 'ssp_bindings.dart';
import 'stream_update.dart';

/// Dart handle to a native `ssp` circuit, bound via the `ssp-ffi` C ABI.
///
/// Mirrors the JS `Sp00kyProcessor` surface that `StreamProcessorService`
/// consumes. All data crosses the boundary as JSON strings; results arrive as
/// an `{"ok": ...}` / `{"err": ...}` envelope which [_decode] unwraps.
class StreamProcessor implements Finalizable {
  StreamProcessor._(this._b, this._ptr) {
    _finalizer.attach(this, _ptr.cast(), detach: this);
  }

  /// Open the native library and create a fresh processor.
  factory StreamProcessor.create([SspBindings? bindings]) {
    final b = bindings ?? SspBindings.open();
    final ptr = b.sspNew();
    if (ptr == nullptr) {
      throw SspException('ssp_new returned null (native init failed)');
    }
    return StreamProcessor._(b, ptr);
  }

  final SspBindings _b;
  Pointer<Void> _ptr;
  bool _disposed = false;

  late final NativeFinalizer _finalizer = NativeFinalizer(_b.library
      .lookup<NativeFunction<Void Function(Pointer<Void>)>>('ssp_free'));

  /// Ingest one record change, returning any affected view updates.
  List<StreamUpdate> ingest(
      String table, String op, String id, Map<String, dynamic> record) {
    final t = table.toNativeUtf8();
    final o = op.toNativeUtf8();
    final i = id.toNativeUtf8();
    final r = jsonEncode(record).toNativeUtf8();
    try {
      final decoded = _decode(_b.sspIngest(_ptr, t, o, i, r)) as List<dynamic>;
      return decoded
          .map((e) => StreamUpdate.fromWasm(e as Map<String, dynamic>, op: op))
          .toList();
    } finally {
      calloc.free(t);
      calloc.free(o);
      calloc.free(i);
      calloc.free(r);
    }
  }

  /// Register a materialized view, returning its initial snapshot.
  StreamUpdate? registerView(Map<String, dynamic> config) {
    final c = jsonEncode(config).toNativeUtf8();
    try {
      final decoded = _decode(_b.sspRegisterView(_ptr, c));
      if (decoded == null) return null;
      return StreamUpdate.fromWasm(decoded as Map<String, dynamic>);
    } finally {
      calloc.free(c);
    }
  }

  /// Seed a table's `PERMISSIONS FOR select WHERE <expr>` text on the circuit.
  ///
  /// Required before [registerView] for a real table, since the circuit is
  /// default-deny. Seed from the schema during init (mirrors the SSP server).
  void setPermission(String table, String whereText) {
    final t = table.toNativeUtf8();
    final w = whereText.toNativeUtf8();
    try {
      _decode(_b.sspSetPermission(_ptr, t, w));
    } finally {
      calloc.free(t);
      calloc.free(w);
    }
  }

  /// Unregister a view by id.
  void unregisterView(String id) {
    final i = id.toNativeUtf8();
    try {
      _decode(_b.sspUnregisterView(_ptr, i));
    } finally {
      calloc.free(i);
    }
  }

  /// Serialize the current circuit state to a JSON string.
  String saveState() => _decode(_b.sspSaveState(_ptr)) as String;

  /// Restore circuit state from a JSON string.
  void loadState(String state) {
    final s = state.toNativeUtf8();
    try {
      _decode(_b.sspLoadState(_ptr, s));
    } finally {
      calloc.free(s);
    }
  }

  /// Free the native processor. Safe to call multiple times.
  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _finalizer.detach(this);
    _b.sspFree(_ptr);
    _ptr = nullptr;
  }

  /// Copy out the Rust-owned result string, free it, and unwrap the envelope.
  dynamic _decode(Pointer<Utf8> ret) {
    if (ret == nullptr) {
      throw SspException('native call returned null pointer');
    }
    String jsonStr;
    try {
      jsonStr = ret.toDartString();
    } finally {
      _b.sspStringFree(ret);
    }
    final env = jsonDecode(jsonStr) as Map<String, dynamic>;
    if (env.containsKey('err')) {
      throw SspException(env['err'] as String);
    }
    return env['ok'];
  }
}
