import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_core/flutter_core.dart';
import 'package:flutter_core/flutter_core_platform_interface.dart';
import 'package:flutter_core/flutter_core_method_channel.dart';
import 'package:plugin_platform_interface/plugin_platform_interface.dart';

class MockFlutterCorePlatform
    with MockPlatformInterfaceMixin
    implements FlutterCorePlatform {

  @override
  Future<String?> getPlatformVersion() => Future.value('42');
}

void main() {
  final FlutterCorePlatform initialPlatform = FlutterCorePlatform.instance;

  test('$MethodChannelFlutterCore is the default instance', () {
    expect(initialPlatform, isInstanceOf<MethodChannelFlutterCore>());
  });

  test('getPlatformVersion', () async {
    FlutterCore flutterCorePlugin = FlutterCore();
    MockFlutterCorePlatform fakePlatform = MockFlutterCorePlatform();
    FlutterCorePlatform.instance = fakePlatform;

    expect(await flutterCorePlugin.getPlatformVersion(), '42');
  });
}
