#include "include/surrealdb/surrealdb_plugin_c_api.h"

#include <flutter/plugin_registrar_windows.h>

#include "surrealdb_plugin.h"

void SurrealdbPluginCApiRegisterWithRegistrar(
    FlutterDesktopPluginRegistrarRef registrar) {
  surrealdb::SurrealdbPlugin::RegisterWithRegistrar(
      flutter::PluginRegistrarManager::GetInstance()
          ->GetRegistrar<flutter::PluginRegistrarWindows>(registrar));
}
