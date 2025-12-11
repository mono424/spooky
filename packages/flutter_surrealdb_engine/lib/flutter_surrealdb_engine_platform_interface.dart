import 'package:plugin_platform_interface/plugin_platform_interface.dart';

import 'flutter_surrealdb_engine_method_channel.dart';

abstract class FlutterSurrealdbEnginePlatform extends PlatformInterface {
  /// Constructs a FlutterSurrealdbEnginePlatform.
  FlutterSurrealdbEnginePlatform() : super(token: _token);

  static final Object _token = Object();

  static FlutterSurrealdbEnginePlatform _instance =
      MethodChannelFlutterSurrealdbEngine();

  /// The default instance of [FlutterSurrealdbEnginePlatform] to use.
  ///
  /// Defaults to [MethodChannelFlutterSurrealdbEngine].
  static FlutterSurrealdbEnginePlatform get instance => _instance;

  /// Platform-specific implementations should set this with their own
  /// platform-specific class that extends [FlutterSurrealdbEnginePlatform] when
  /// they register themselves.
  static set instance(FlutterSurrealdbEnginePlatform instance) {
    PlatformInterface.verifyToken(instance, _token);
    _instance = instance;
  }

  Future<String?> getPlatformVersion() {
    throw UnimplementedError('platformVersion() has not been implemented.');
  }
}
