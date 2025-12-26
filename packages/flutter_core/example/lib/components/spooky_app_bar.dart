import 'package:flutter/material.dart';

class SpookyAppBar extends StatelessWidget implements PreferredSizeWidget {
  final VoidCallback onDisconnect;
  final VoidCallback onLogout;
  final bool isLoggedIn;

  const SpookyAppBar({
    super.key,
    required this.onDisconnect,
    required this.onLogout,
    required this.isLoggedIn,
  });

  @override
  Widget build(BuildContext context) {
    return AppBar(
      title: const Text('SpookyClient Testbed'),
      backgroundColor: Colors.deepPurple,
      foregroundColor: Colors.white,
      actions: [
        IconButton(
          icon: const Icon(Icons.power_settings_new),
          tooltip: "Disconnect / Close DB",
          onPressed: onDisconnect,
        ),
        if (isLoggedIn)
          IconButton(icon: const Icon(Icons.logout), onPressed: onLogout),
      ],
    );
  }

  @override
  Size get preferredSize => const Size.fromHeight(kToolbarHeight);
}
