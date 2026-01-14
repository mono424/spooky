//
//  Generated file. Do not edit.
//

// clang-format off

#include "generated_plugin_registrant.h"

#include <flutter_core/flutter_core_plugin_c_api.h>
#include <flutter_surrealdb_engine/surrealdb_plugin_c_api.h>

void RegisterPlugins(flutter::PluginRegistry* registry) {
  FlutterCorePluginCApiRegisterWithRegistrar(
      registry->GetRegistrarForPlugin("FlutterCorePluginCApi"));
  SurrealdbPluginCApiRegisterWithRegistrar(
      registry->GetRegistrarForPlugin("SurrealdbPluginCApi"));
}
