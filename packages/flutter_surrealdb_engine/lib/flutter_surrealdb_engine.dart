import 'surrealdb_platform_interface.dart';

export 'src/rust/api/client.dart';
export 'src/rust/frb_generated.dart' show RustLib;

class Surrealdb {
  Future<String?> getPlatformVersion() {
    return SurrealdbPlatform.instance.getPlatformVersion();
  }
}
