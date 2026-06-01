import 'dart:ffi';

import 'package:ffi/ffi.dart';

import 'ssp_library.dart';

// Native C signatures (see packages/ssp-ffi/src/lib.rs).
typedef _NewNative = Pointer<Void> Function();
typedef _FreeNative = Void Function(Pointer<Void>);
typedef _FreeDart = void Function(Pointer<Void>);
typedef _StringFreeNative = Void Function(Pointer<Utf8>);
typedef _StringFreeDart = void Function(Pointer<Utf8>);

typedef _IngestNative = Pointer<Utf8> Function(
    Pointer<Void>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>);
typedef _IngestDart = Pointer<Utf8> Function(
    Pointer<Void>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>, Pointer<Utf8>);

typedef _OnePtrNative = Pointer<Utf8> Function(Pointer<Void>, Pointer<Utf8>);
typedef _OnePtrDart = Pointer<Utf8> Function(Pointer<Void>, Pointer<Utf8>);

typedef _SaveNative = Pointer<Utf8> Function(Pointer<Void>);
typedef _SaveDart = Pointer<Utf8> Function(Pointer<Void>);

typedef _TwoPtrNative = Pointer<Utf8> Function(
    Pointer<Void>, Pointer<Utf8>, Pointer<Utf8>);
typedef _TwoPtrDart = Pointer<Utf8> Function(
    Pointer<Void>, Pointer<Utf8>, Pointer<Utf8>);

/// Thin lookup wrapper around the `ssp-ffi` C ABI.
class SspBindings {
  SspBindings(this._lib)
      : sspNew = _lib.lookupFunction<_NewNative, _NewNative>('ssp_new'),
        sspFree = _lib.lookupFunction<_FreeNative, _FreeDart>('ssp_free'),
        sspStringFree = _lib.lookupFunction<_StringFreeNative, _StringFreeDart>(
            'ssp_string_free'),
        sspIngest =
            _lib.lookupFunction<_IngestNative, _IngestDart>('ssp_ingest'),
        sspRegisterView = _lib
            .lookupFunction<_OnePtrNative, _OnePtrDart>('ssp_register_view'),
        sspUnregisterView = _lib
            .lookupFunction<_OnePtrNative, _OnePtrDart>('ssp_unregister_view'),
        sspSaveState =
            _lib.lookupFunction<_SaveNative, _SaveDart>('ssp_save_state'),
        sspLoadState =
            _lib.lookupFunction<_OnePtrNative, _OnePtrDart>('ssp_load_state'),
        sspSetPermission = _lib
            .lookupFunction<_TwoPtrNative, _TwoPtrDart>('ssp_set_permission');

  factory SspBindings.open() => SspBindings(openSspLibrary());

  final DynamicLibrary _lib;

  /// The underlying library handle, used to attach a [NativeFinalizer].
  DynamicLibrary get library => _lib;

  final Pointer<Void> Function() sspNew;
  final _FreeDart sspFree;
  final _StringFreeDart sspStringFree;
  final _IngestDart sspIngest;
  final _OnePtrDart sspRegisterView;
  final _OnePtrDart sspUnregisterView;
  final _SaveDart sspSaveState;
  final _OnePtrDart sspLoadState;
  final _TwoPtrDart sspSetPermission;
}
