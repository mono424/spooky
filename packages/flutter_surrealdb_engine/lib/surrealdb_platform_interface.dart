import 'package:plugin_platform_interface/plugin_platform_interface.dart';

import 'surrealdb_method_channel.dart';

abstract class SurrealdbPlatform extends PlatformInterface {
  /// Constructs a SurrealdbPlatform.
  SurrealdbPlatform() : super(token: _token);

  static final Object _token = Object();

  static SurrealdbPlatform _instance = MethodChannelSurrealdb();

  /// The default instance of [SurrealdbPlatform] to use.
  ///
  /// Defaults to [MethodChannelSurrealdb].
  static SurrealdbPlatform get instance => _instance;

  /// Platform-specific implementations should set this with their own
  /// platform-specific class that extends [SurrealdbPlatform] when
  /// they register themselves.
  static set instance(SurrealdbPlatform instance) {
    PlatformInterface.verifyToken(instance, _token);
    _instance = instance;
  }

  Future<String?> getPlatformVersion() {
    throw UnimplementedError('platformVersion() has not been implemented.');
  }
}
