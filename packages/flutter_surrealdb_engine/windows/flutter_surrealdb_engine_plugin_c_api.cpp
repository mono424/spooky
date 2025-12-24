#include "include/flutter_surrealdb_engine/flutter_surrealdb_engine_plugin_c_api.h"

#include <flutter/plugin_registrar_windows.h>

#include "flutter_surrealdb_engine_plugin.h"

void FlutterSurrealdbEnginePluginCApiRegisterWithRegistrar(
    FlutterDesktopPluginRegistrarRef registrar) {
  flutter_surrealdb_engine::FlutterSurrealdbEnginePlugin::RegisterWithRegistrar(
      flutter::PluginRegistrarManager::GetInstance()
          ->GetRegistrar<flutter::PluginRegistrarWindows>(registrar));
}
