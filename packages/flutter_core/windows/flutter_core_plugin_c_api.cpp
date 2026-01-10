#include "include/flutter_core/flutter_core_plugin_c_api.h"

#include <flutter/plugin_registrar_windows.h>

#include "flutter_core_plugin.h"

void FlutterCorePluginCApiRegisterWithRegistrar(
    FlutterDesktopPluginRegistrarRef registrar) {
  flutter_core::FlutterCorePlugin::RegisterWithRegistrar(
      flutter::PluginRegistrarManager::GetInstance()
          ->GetRegistrar<flutter::PluginRegistrarWindows>(registrar));
}
