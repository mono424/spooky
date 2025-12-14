import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine_platform_interface.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine_method_channel.dart';
import 'package:plugin_platform_interface/plugin_platform_interface.dart';

class MockFlutterSurrealdbEnginePlatform
    with MockPlatformInterfaceMixin
    implements FlutterSurrealdbEnginePlatform {
  @override
  Future<String?> getPlatformVersion() => Future.value('42');
}

void main() {
  final FlutterSurrealdbEnginePlatform initialPlatform =
      FlutterSurrealdbEnginePlatform.instance;

  test('$MethodChannelFlutterSurrealdbEngine is the default instance', () {
    expect(
      initialPlatform,
      isInstanceOf<MethodChannelFlutterSurrealdbEngine>(),
    );
  });

  test('getPlatformVersion', () async {
    FlutterSurrealdbEngine flutterSurrealdbEnginePlugin =
        FlutterSurrealdbEngine();
    MockFlutterSurrealdbEnginePlatform fakePlatform =
        MockFlutterSurrealdbEnginePlatform();
    FlutterSurrealdbEnginePlatform.instance = fakePlatform;

    expect(await flutterSurrealdbEnginePlugin.getPlatformVersion(), '42');
  });
}
