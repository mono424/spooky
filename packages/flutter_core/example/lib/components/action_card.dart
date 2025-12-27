import 'package:flutter/material.dart';
import 'package:flutter_core_example/core/theme.dart';
import 'package:google_fonts/google_fonts.dart';

class ActionCard extends StatelessWidget {
  final String title;
  final IconData icon;
  final VoidCallback onTap;
  final Color? color;
  final Color? textColor;

  const ActionCard({
    super.key,
    required this.title,
    required this.icon,
    required this.onTap,
    this.color,
    this.textColor,
  });

  @override
  Widget build(BuildContext context) {
    return InkWell(
      onTap: onTap,
      child: Container(
        width: 160,
        height: 120,
        decoration: BoxDecoration(
          color: SpookyColors.background,
          border: Border.all(color: SpookyColors.white),
        ),
        child: Column(
          mainAxisAlignment: MainAxisAlignment.center,
          children: [
            Icon(icon, size: 32, color: SpookyColors.white),
            const SizedBox(height: 8),
            Text(
              title,
              style: GoogleFonts.spaceMono(
                fontWeight: FontWeight.w600,
                color: SpookyColors.white,
              ),
            ),
          ],
        ),
      ),
    );
  }
}
