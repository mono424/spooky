import 'package:flutter/material.dart';
import 'package:flutter_core_example/core/theme.dart';
import 'package:google_fonts/google_fonts.dart';

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
      title: Text(
        '[ SPOOKY_CLIENT_EXAMPLE ]',
        style: GoogleFonts.spaceMono(
          fontWeight: FontWeight.bold,
          fontSize: 16,
          letterSpacing: 1.2,
        ),
      ),
      backgroundColor: SpookyColors.background,
      foregroundColor: SpookyColors.white,
      elevation: 0,
      centerTitle: true,
      actions: [
        Padding(
          padding: const EdgeInsets.only(right: 16.0),
          child: IconButton(
            icon: const Icon(Icons.power_settings_new),
            tooltip: "Disconnect / Close DB",
            onPressed: onDisconnect,
          ),
        ),

        if (isLoggedIn)
          IconButton(icon: const Icon(Icons.logout), onPressed: onLogout),
      ],
    );
  }

  @override
  Size get preferredSize => const Size.fromHeight(kToolbarHeight);
}
