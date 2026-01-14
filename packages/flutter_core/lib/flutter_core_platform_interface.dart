import 'package:plugin_platform_interface/plugin_platform_interface.dart';

import 'flutter_core_method_channel.dart';

abstract class FlutterCorePlatform extends PlatformInterface {
  /// Constructs a FlutterCorePlatform.
  FlutterCorePlatform() : super(token: _token);

  static final Object _token = Object();

  static FlutterCorePlatform _instance = MethodChannelFlutterCore();

  /// The default instance of [FlutterCorePlatform] to use.
  ///
  /// Defaults to [MethodChannelFlutterCore].
  static FlutterCorePlatform get instance => _instance;

  /// Platform-specific implementations should set this with their own
  /// platform-specific class that extends [FlutterCorePlatform] when
  /// they register themselves.
  static set instance(FlutterCorePlatform instance) {
    PlatformInterface.verifyToken(instance, _token);
    _instance = instance;
  }

  Future<String?> getPlatformVersion() {
    throw UnimplementedError('platformVersion() has not been implemented.');
  }
}
