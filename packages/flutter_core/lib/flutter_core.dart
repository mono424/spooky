import 'flutter_core_platform_interface.dart';

class FlutterCore {
  Future<String?> getPlatformVersion() {
    return FlutterCorePlatform.instance.getPlatformVersion();
  }

  int? add(int a, int b) {
    return FlutterCorePlatform.instance.add(a, b);
  }
}
