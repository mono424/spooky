// coverage:ignore-file
// GENERATED CODE - DO NOT MODIFY BY HAND
// ignore_for_file: type=lint
// ignore_for_file: unused_element, deprecated_member_use, deprecated_member_use_from_same_package, use_function_type_syntax_for_parameters, unnecessary_const, avoid_init_to_null, invalid_override_different_default_values_named, prefer_expression_function_bodies, annotate_overrides, invalid_annotation_target, unnecessary_question_mark

part of 'client.dart';

// **************************************************************************
// FreezedGenerator
// **************************************************************************

T _$identity<T>(T value) => value;

final _privateConstructorUsedError = UnsupportedError(
  'It seems like you constructed your class using `MyClass._()`. This constructor is only meant to be used by freezed and you are not supposed to need it nor use it.\nPlease check the documentation here for more information: https://github.com/rrousselGit/freezed#adding-getters-and-methods-to-our-models',
);

/// @nodoc
mixin _$StorageMode {
  @optionalTypeArgs
  TResult when<TResult extends Object?>({
    required TResult Function() memory,
    required TResult Function(String path) disk,
    required TResult Function(String url) remote,
    required TResult Function(String path, int port) devSidecar,
  }) => throw _privateConstructorUsedError;
  @optionalTypeArgs
  TResult? whenOrNull<TResult extends Object?>({
    TResult? Function()? memory,
    TResult? Function(String path)? disk,
    TResult? Function(String url)? remote,
    TResult? Function(String path, int port)? devSidecar,
  }) => throw _privateConstructorUsedError;
  @optionalTypeArgs
  TResult maybeWhen<TResult extends Object?>({
    TResult Function()? memory,
    TResult Function(String path)? disk,
    TResult Function(String url)? remote,
    TResult Function(String path, int port)? devSidecar,
    required TResult orElse(),
  }) => throw _privateConstructorUsedError;
  @optionalTypeArgs
  TResult map<TResult extends Object?>({
    required TResult Function(StorageMode_Memory value) memory,
    required TResult Function(StorageMode_Disk value) disk,
    required TResult Function(StorageMode_Remote value) remote,
    required TResult Function(StorageMode_DevSidecar value) devSidecar,
  }) => throw _privateConstructorUsedError;
  @optionalTypeArgs
  TResult? mapOrNull<TResult extends Object?>({
    TResult? Function(StorageMode_Memory value)? memory,
    TResult? Function(StorageMode_Disk value)? disk,
    TResult? Function(StorageMode_Remote value)? remote,
    TResult? Function(StorageMode_DevSidecar value)? devSidecar,
  }) => throw _privateConstructorUsedError;
  @optionalTypeArgs
  TResult maybeMap<TResult extends Object?>({
    TResult Function(StorageMode_Memory value)? memory,
    TResult Function(StorageMode_Disk value)? disk,
    TResult Function(StorageMode_Remote value)? remote,
    TResult Function(StorageMode_DevSidecar value)? devSidecar,
    required TResult orElse(),
  }) => throw _privateConstructorUsedError;
}

/// @nodoc
abstract class $StorageModeCopyWith<$Res> {
  factory $StorageModeCopyWith(
    StorageMode value,
    $Res Function(StorageMode) then,
  ) = _$StorageModeCopyWithImpl<$Res, StorageMode>;
}

/// @nodoc
class _$StorageModeCopyWithImpl<$Res, $Val extends StorageMode>
    implements $StorageModeCopyWith<$Res> {
  _$StorageModeCopyWithImpl(this._value, this._then);

  // ignore: unused_field
  final $Val _value;
  // ignore: unused_field
  final $Res Function($Val) _then;

  /// Create a copy of StorageMode
  /// with the given fields replaced by the non-null parameter values.
}

/// @nodoc
abstract class _$$StorageMode_MemoryImplCopyWith<$Res> {
  factory _$$StorageMode_MemoryImplCopyWith(
    _$StorageMode_MemoryImpl value,
    $Res Function(_$StorageMode_MemoryImpl) then,
  ) = __$$StorageMode_MemoryImplCopyWithImpl<$Res>;
}

/// @nodoc
class __$$StorageMode_MemoryImplCopyWithImpl<$Res>
    extends _$StorageModeCopyWithImpl<$Res, _$StorageMode_MemoryImpl>
    implements _$$StorageMode_MemoryImplCopyWith<$Res> {
  __$$StorageMode_MemoryImplCopyWithImpl(
    _$StorageMode_MemoryImpl _value,
    $Res Function(_$StorageMode_MemoryImpl) _then,
  ) : super(_value, _then);

  /// Create a copy of StorageMode
  /// with the given fields replaced by the non-null parameter values.
}

/// @nodoc

class _$StorageMode_MemoryImpl extends StorageMode_Memory {
  const _$StorageMode_MemoryImpl() : super._();

  @override
  String toString() {
    return 'StorageMode.memory()';
  }

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType && other is _$StorageMode_MemoryImpl);
  }

  @override
  int get hashCode => runtimeType.hashCode;

  @override
  @optionalTypeArgs
  TResult when<TResult extends Object?>({
    required TResult Function() memory,
    required TResult Function(String path) disk,
    required TResult Function(String url) remote,
    required TResult Function(String path, int port) devSidecar,
  }) {
    return memory();
  }

  @override
  @optionalTypeArgs
  TResult? whenOrNull<TResult extends Object?>({
    TResult? Function()? memory,
    TResult? Function(String path)? disk,
    TResult? Function(String url)? remote,
    TResult? Function(String path, int port)? devSidecar,
  }) {
    return memory?.call();
  }

  @override
  @optionalTypeArgs
  TResult maybeWhen<TResult extends Object?>({
    TResult Function()? memory,
    TResult Function(String path)? disk,
    TResult Function(String url)? remote,
    TResult Function(String path, int port)? devSidecar,
    required TResult orElse(),
  }) {
    if (memory != null) {
      return memory();
    }
    return orElse();
  }

  @override
  @optionalTypeArgs
  TResult map<TResult extends Object?>({
    required TResult Function(StorageMode_Memory value) memory,
    required TResult Function(StorageMode_Disk value) disk,
    required TResult Function(StorageMode_Remote value) remote,
    required TResult Function(StorageMode_DevSidecar value) devSidecar,
  }) {
    return memory(this);
  }

  @override
  @optionalTypeArgs
  TResult? mapOrNull<TResult extends Object?>({
    TResult? Function(StorageMode_Memory value)? memory,
    TResult? Function(StorageMode_Disk value)? disk,
    TResult? Function(StorageMode_Remote value)? remote,
    TResult? Function(StorageMode_DevSidecar value)? devSidecar,
  }) {
    return memory?.call(this);
  }

  @override
  @optionalTypeArgs
  TResult maybeMap<TResult extends Object?>({
    TResult Function(StorageMode_Memory value)? memory,
    TResult Function(StorageMode_Disk value)? disk,
    TResult Function(StorageMode_Remote value)? remote,
    TResult Function(StorageMode_DevSidecar value)? devSidecar,
    required TResult orElse(),
  }) {
    if (memory != null) {
      return memory(this);
    }
    return orElse();
  }
}

abstract class StorageMode_Memory extends StorageMode {
  const factory StorageMode_Memory() = _$StorageMode_MemoryImpl;
  const StorageMode_Memory._() : super._();
}

/// @nodoc
abstract class _$$StorageMode_DiskImplCopyWith<$Res> {
  factory _$$StorageMode_DiskImplCopyWith(
    _$StorageMode_DiskImpl value,
    $Res Function(_$StorageMode_DiskImpl) then,
  ) = __$$StorageMode_DiskImplCopyWithImpl<$Res>;
  @useResult
  $Res call({String path});
}

/// @nodoc
class __$$StorageMode_DiskImplCopyWithImpl<$Res>
    extends _$StorageModeCopyWithImpl<$Res, _$StorageMode_DiskImpl>
    implements _$$StorageMode_DiskImplCopyWith<$Res> {
  __$$StorageMode_DiskImplCopyWithImpl(
    _$StorageMode_DiskImpl _value,
    $Res Function(_$StorageMode_DiskImpl) _then,
  ) : super(_value, _then);

  /// Create a copy of StorageMode
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  @override
  $Res call({Object? path = null}) {
    return _then(
      _$StorageMode_DiskImpl(
        path: null == path
            ? _value.path
            : path // ignore: cast_nullable_to_non_nullable
                  as String,
      ),
    );
  }
}

/// @nodoc

class _$StorageMode_DiskImpl extends StorageMode_Disk {
  const _$StorageMode_DiskImpl({required this.path}) : super._();

  @override
  final String path;

  @override
  String toString() {
    return 'StorageMode.disk(path: $path)';
  }

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is _$StorageMode_DiskImpl &&
            (identical(other.path, path) || other.path == path));
  }

  @override
  int get hashCode => Object.hash(runtimeType, path);

  /// Create a copy of StorageMode
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @override
  @pragma('vm:prefer-inline')
  _$$StorageMode_DiskImplCopyWith<_$StorageMode_DiskImpl> get copyWith =>
      __$$StorageMode_DiskImplCopyWithImpl<_$StorageMode_DiskImpl>(
        this,
        _$identity,
      );

  @override
  @optionalTypeArgs
  TResult when<TResult extends Object?>({
    required TResult Function() memory,
    required TResult Function(String path) disk,
    required TResult Function(String url) remote,
    required TResult Function(String path, int port) devSidecar,
  }) {
    return disk(path);
  }

  @override
  @optionalTypeArgs
  TResult? whenOrNull<TResult extends Object?>({
    TResult? Function()? memory,
    TResult? Function(String path)? disk,
    TResult? Function(String url)? remote,
    TResult? Function(String path, int port)? devSidecar,
  }) {
    return disk?.call(path);
  }

  @override
  @optionalTypeArgs
  TResult maybeWhen<TResult extends Object?>({
    TResult Function()? memory,
    TResult Function(String path)? disk,
    TResult Function(String url)? remote,
    TResult Function(String path, int port)? devSidecar,
    required TResult orElse(),
  }) {
    if (disk != null) {
      return disk(path);
    }
    return orElse();
  }

  @override
  @optionalTypeArgs
  TResult map<TResult extends Object?>({
    required TResult Function(StorageMode_Memory value) memory,
    required TResult Function(StorageMode_Disk value) disk,
    required TResult Function(StorageMode_Remote value) remote,
    required TResult Function(StorageMode_DevSidecar value) devSidecar,
  }) {
    return disk(this);
  }

  @override
  @optionalTypeArgs
  TResult? mapOrNull<TResult extends Object?>({
    TResult? Function(StorageMode_Memory value)? memory,
    TResult? Function(StorageMode_Disk value)? disk,
    TResult? Function(StorageMode_Remote value)? remote,
    TResult? Function(StorageMode_DevSidecar value)? devSidecar,
  }) {
    return disk?.call(this);
  }

  @override
  @optionalTypeArgs
  TResult maybeMap<TResult extends Object?>({
    TResult Function(StorageMode_Memory value)? memory,
    TResult Function(StorageMode_Disk value)? disk,
    TResult Function(StorageMode_Remote value)? remote,
    TResult Function(StorageMode_DevSidecar value)? devSidecar,
    required TResult orElse(),
  }) {
    if (disk != null) {
      return disk(this);
    }
    return orElse();
  }
}

abstract class StorageMode_Disk extends StorageMode {
  const factory StorageMode_Disk({required final String path}) =
      _$StorageMode_DiskImpl;
  const StorageMode_Disk._() : super._();

  String get path;

  /// Create a copy of StorageMode
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  _$$StorageMode_DiskImplCopyWith<_$StorageMode_DiskImpl> get copyWith =>
      throw _privateConstructorUsedError;
}

/// @nodoc
abstract class _$$StorageMode_RemoteImplCopyWith<$Res> {
  factory _$$StorageMode_RemoteImplCopyWith(
    _$StorageMode_RemoteImpl value,
    $Res Function(_$StorageMode_RemoteImpl) then,
  ) = __$$StorageMode_RemoteImplCopyWithImpl<$Res>;
  @useResult
  $Res call({String url});
}

/// @nodoc
class __$$StorageMode_RemoteImplCopyWithImpl<$Res>
    extends _$StorageModeCopyWithImpl<$Res, _$StorageMode_RemoteImpl>
    implements _$$StorageMode_RemoteImplCopyWith<$Res> {
  __$$StorageMode_RemoteImplCopyWithImpl(
    _$StorageMode_RemoteImpl _value,
    $Res Function(_$StorageMode_RemoteImpl) _then,
  ) : super(_value, _then);

  /// Create a copy of StorageMode
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  @override
  $Res call({Object? url = null}) {
    return _then(
      _$StorageMode_RemoteImpl(
        url: null == url
            ? _value.url
            : url // ignore: cast_nullable_to_non_nullable
                  as String,
      ),
    );
  }
}

/// @nodoc

class _$StorageMode_RemoteImpl extends StorageMode_Remote {
  const _$StorageMode_RemoteImpl({required this.url}) : super._();

  @override
  final String url;

  @override
  String toString() {
    return 'StorageMode.remote(url: $url)';
  }

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is _$StorageMode_RemoteImpl &&
            (identical(other.url, url) || other.url == url));
  }

  @override
  int get hashCode => Object.hash(runtimeType, url);

  /// Create a copy of StorageMode
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @override
  @pragma('vm:prefer-inline')
  _$$StorageMode_RemoteImplCopyWith<_$StorageMode_RemoteImpl> get copyWith =>
      __$$StorageMode_RemoteImplCopyWithImpl<_$StorageMode_RemoteImpl>(
        this,
        _$identity,
      );

  @override
  @optionalTypeArgs
  TResult when<TResult extends Object?>({
    required TResult Function() memory,
    required TResult Function(String path) disk,
    required TResult Function(String url) remote,
    required TResult Function(String path, int port) devSidecar,
  }) {
    return remote(url);
  }

  @override
  @optionalTypeArgs
  TResult? whenOrNull<TResult extends Object?>({
    TResult? Function()? memory,
    TResult? Function(String path)? disk,
    TResult? Function(String url)? remote,
    TResult? Function(String path, int port)? devSidecar,
  }) {
    return remote?.call(url);
  }

  @override
  @optionalTypeArgs
  TResult maybeWhen<TResult extends Object?>({
    TResult Function()? memory,
    TResult Function(String path)? disk,
    TResult Function(String url)? remote,
    TResult Function(String path, int port)? devSidecar,
    required TResult orElse(),
  }) {
    if (remote != null) {
      return remote(url);
    }
    return orElse();
  }

  @override
  @optionalTypeArgs
  TResult map<TResult extends Object?>({
    required TResult Function(StorageMode_Memory value) memory,
    required TResult Function(StorageMode_Disk value) disk,
    required TResult Function(StorageMode_Remote value) remote,
    required TResult Function(StorageMode_DevSidecar value) devSidecar,
  }) {
    return remote(this);
  }

  @override
  @optionalTypeArgs
  TResult? mapOrNull<TResult extends Object?>({
    TResult? Function(StorageMode_Memory value)? memory,
    TResult? Function(StorageMode_Disk value)? disk,
    TResult? Function(StorageMode_Remote value)? remote,
    TResult? Function(StorageMode_DevSidecar value)? devSidecar,
  }) {
    return remote?.call(this);
  }

  @override
  @optionalTypeArgs
  TResult maybeMap<TResult extends Object?>({
    TResult Function(StorageMode_Memory value)? memory,
    TResult Function(StorageMode_Disk value)? disk,
    TResult Function(StorageMode_Remote value)? remote,
    TResult Function(StorageMode_DevSidecar value)? devSidecar,
    required TResult orElse(),
  }) {
    if (remote != null) {
      return remote(this);
    }
    return orElse();
  }
}

abstract class StorageMode_Remote extends StorageMode {
  const factory StorageMode_Remote({required final String url}) =
      _$StorageMode_RemoteImpl;
  const StorageMode_Remote._() : super._();

  String get url;

  /// Create a copy of StorageMode
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  _$$StorageMode_RemoteImplCopyWith<_$StorageMode_RemoteImpl> get copyWith =>
      throw _privateConstructorUsedError;
}

/// @nodoc
abstract class _$$StorageMode_DevSidecarImplCopyWith<$Res> {
  factory _$$StorageMode_DevSidecarImplCopyWith(
    _$StorageMode_DevSidecarImpl value,
    $Res Function(_$StorageMode_DevSidecarImpl) then,
  ) = __$$StorageMode_DevSidecarImplCopyWithImpl<$Res>;
  @useResult
  $Res call({String path, int port});
}

/// @nodoc
class __$$StorageMode_DevSidecarImplCopyWithImpl<$Res>
    extends _$StorageModeCopyWithImpl<$Res, _$StorageMode_DevSidecarImpl>
    implements _$$StorageMode_DevSidecarImplCopyWith<$Res> {
  __$$StorageMode_DevSidecarImplCopyWithImpl(
    _$StorageMode_DevSidecarImpl _value,
    $Res Function(_$StorageMode_DevSidecarImpl) _then,
  ) : super(_value, _then);

  /// Create a copy of StorageMode
  /// with the given fields replaced by the non-null parameter values.
  @pragma('vm:prefer-inline')
  @override
  $Res call({Object? path = null, Object? port = null}) {
    return _then(
      _$StorageMode_DevSidecarImpl(
        path: null == path
            ? _value.path
            : path // ignore: cast_nullable_to_non_nullable
                  as String,
        port: null == port
            ? _value.port
            : port // ignore: cast_nullable_to_non_nullable
                  as int,
      ),
    );
  }
}

/// @nodoc

class _$StorageMode_DevSidecarImpl extends StorageMode_DevSidecar {
  const _$StorageMode_DevSidecarImpl({required this.path, required this.port})
    : super._();

  @override
  final String path;
  @override
  final int port;

  @override
  String toString() {
    return 'StorageMode.devSidecar(path: $path, port: $port)';
  }

  @override
  bool operator ==(Object other) {
    return identical(this, other) ||
        (other.runtimeType == runtimeType &&
            other is _$StorageMode_DevSidecarImpl &&
            (identical(other.path, path) || other.path == path) &&
            (identical(other.port, port) || other.port == port));
  }

  @override
  int get hashCode => Object.hash(runtimeType, path, port);

  /// Create a copy of StorageMode
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  @override
  @pragma('vm:prefer-inline')
  _$$StorageMode_DevSidecarImplCopyWith<_$StorageMode_DevSidecarImpl>
  get copyWith =>
      __$$StorageMode_DevSidecarImplCopyWithImpl<_$StorageMode_DevSidecarImpl>(
        this,
        _$identity,
      );

  @override
  @optionalTypeArgs
  TResult when<TResult extends Object?>({
    required TResult Function() memory,
    required TResult Function(String path) disk,
    required TResult Function(String url) remote,
    required TResult Function(String path, int port) devSidecar,
  }) {
    return devSidecar(path, port);
  }

  @override
  @optionalTypeArgs
  TResult? whenOrNull<TResult extends Object?>({
    TResult? Function()? memory,
    TResult? Function(String path)? disk,
    TResult? Function(String url)? remote,
    TResult? Function(String path, int port)? devSidecar,
  }) {
    return devSidecar?.call(path, port);
  }

  @override
  @optionalTypeArgs
  TResult maybeWhen<TResult extends Object?>({
    TResult Function()? memory,
    TResult Function(String path)? disk,
    TResult Function(String url)? remote,
    TResult Function(String path, int port)? devSidecar,
    required TResult orElse(),
  }) {
    if (devSidecar != null) {
      return devSidecar(path, port);
    }
    return orElse();
  }

  @override
  @optionalTypeArgs
  TResult map<TResult extends Object?>({
    required TResult Function(StorageMode_Memory value) memory,
    required TResult Function(StorageMode_Disk value) disk,
    required TResult Function(StorageMode_Remote value) remote,
    required TResult Function(StorageMode_DevSidecar value) devSidecar,
  }) {
    return devSidecar(this);
  }

  @override
  @optionalTypeArgs
  TResult? mapOrNull<TResult extends Object?>({
    TResult? Function(StorageMode_Memory value)? memory,
    TResult? Function(StorageMode_Disk value)? disk,
    TResult? Function(StorageMode_Remote value)? remote,
    TResult? Function(StorageMode_DevSidecar value)? devSidecar,
  }) {
    return devSidecar?.call(this);
  }

  @override
  @optionalTypeArgs
  TResult maybeMap<TResult extends Object?>({
    TResult Function(StorageMode_Memory value)? memory,
    TResult Function(StorageMode_Disk value)? disk,
    TResult Function(StorageMode_Remote value)? remote,
    TResult Function(StorageMode_DevSidecar value)? devSidecar,
    required TResult orElse(),
  }) {
    if (devSidecar != null) {
      return devSidecar(this);
    }
    return orElse();
  }
}

abstract class StorageMode_DevSidecar extends StorageMode {
  const factory StorageMode_DevSidecar({
    required final String path,
    required final int port,
  }) = _$StorageMode_DevSidecarImpl;
  const StorageMode_DevSidecar._() : super._();

  String get path;
  int get port;

  /// Create a copy of StorageMode
  /// with the given fields replaced by the non-null parameter values.
  @JsonKey(includeFromJson: false, includeToJson: false)
  _$$StorageMode_DevSidecarImplCopyWith<_$StorageMode_DevSidecarImpl>
  get copyWith => throw _privateConstructorUsedError;
}
