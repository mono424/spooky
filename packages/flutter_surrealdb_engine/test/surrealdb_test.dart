import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_surrealdb_engine/flutter_surrealdb_engine.dart';
import 'package:flutter_surrealdb_engine/surrealdb_platform_interface.dart';
import 'package:flutter_surrealdb_engine/surrealdb_method_channel.dart';
import 'package:plugin_platform_interface/plugin_platform_interface.dart';

class MockSurrealdbPlatform
    with MockPlatformInterfaceMixin
    implements SurrealdbPlatform {
  @override
  Future<String?> getPlatformVersion() => Future.value('42');
}

void main() {
  final SurrealdbPlatform initialPlatform = SurrealdbPlatform.instance;

  test('$MethodChannelSurrealdb is the default instance', () {
    expect(initialPlatform, isInstanceOf<MethodChannelSurrealdb>());
  });

  test('getPlatformVersion', () async {
    Surrealdb surrealdbPlugin = Surrealdb();
    MockSurrealdbPlatform fakePlatform = MockSurrealdbPlatform();
    SurrealdbPlatform.instance = fakePlatform;

    expect(await surrealdbPlugin.getPlatformVersion(), '42');
  });
}
