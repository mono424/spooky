import 'flutter_surrealdb_engine_platform_interface.dart';

export 'src/rust/lib.dart';
export 'src/rust/frb_generated.dart';

class FlutterSurrealdbEngine {
  Future<String?> getPlatformVersion() {
    return FlutterSurrealdbEnginePlatform.instance.getPlatformVersion();
  }
}
