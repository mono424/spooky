import 'package:flutter/material.dart';
import '../controllers/spooky_controller.dart';
import '../modules/initialization/initialization_view.dart';
import '../modules/auth/auth_view.dart';
import '../modules/chat/chat_dashboard.dart';

class ViewSwitcher extends StatelessWidget {
  final SpookyController controller;
  final Function(String) onError;

  const ViewSwitcher({
    super.key,
    required this.controller,
    required this.onError,
  });

  @override
  Widget build(BuildContext context) {
    if (!controller.isInitialized) {
      return InitializationView(
        namespaceController: controller.namespaceController,
        databaseController: controller.databaseController,
        endpointController: controller.endpointController,
        devSidecarPortController: controller.devSidecarPortController,
        useDevSidecar: controller.useDevSidecar,
        onDevSidecarChanged: controller.toggleDevSidecar,
        onInit: controller.initSpooky,
      );
    }

    if (!controller.isLoggedIn) {
      return AuthView(controller: controller);
    }

    return ChatDashboard(controller: controller);
  }
}

