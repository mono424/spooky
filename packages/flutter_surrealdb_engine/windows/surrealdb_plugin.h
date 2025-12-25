#ifndef FLUTTER_PLUGIN_SURREALDB_PLUGIN_H_
#define FLUTTER_PLUGIN_SURREALDB_PLUGIN_H_

#include <flutter/method_channel.h>
#include <flutter/plugin_registrar_windows.h>

#include <memory>

namespace surrealdb {

class SurrealdbPlugin : public flutter::Plugin {
 public:
  static void RegisterWithRegistrar(flutter::PluginRegistrarWindows *registrar);

  SurrealdbPlugin();

  virtual ~SurrealdbPlugin();

  // Disallow copy and assign.
  SurrealdbPlugin(const SurrealdbPlugin&) = delete;
  SurrealdbPlugin& operator=(const SurrealdbPlugin&) = delete;

  // Called when a method is called on this plugin's channel from Dart.
  void HandleMethodCall(
      const flutter::MethodCall<flutter::EncodableValue> &method_call,
      std::unique_ptr<flutter::MethodResult<flutter::EncodableValue>> result);
};

}  // namespace surrealdb

#endif  // FLUTTER_PLUGIN_SURREALDB_PLUGIN_H_
