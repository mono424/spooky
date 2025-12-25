import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';

import 'surrealdb_platform_interface.dart';

/// An implementation of [SurrealdbPlatform] that uses method channels.
class MethodChannelSurrealdb extends SurrealdbPlatform {
  /// The method channel used to interact with the native platform.
  @visibleForTesting
  final methodChannel = const MethodChannel('surrealdb');

  @override
  Future<String?> getPlatformVersion() async {
    final version = await methodChannel.invokeMethod<String>(
      'getPlatformVersion',
    );
    return version;
  }
}
