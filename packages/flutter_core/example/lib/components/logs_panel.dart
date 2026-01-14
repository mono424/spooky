import 'package:flutter/material.dart';
import 'package:flutter_core_example/core/theme.dart';
import 'package:google_fonts/google_fonts.dart';

class LogsPanel extends StatelessWidget {
  final TextEditingController controller;
  final ScrollController scrollController;

  const LogsPanel({
    super.key,
    required this.controller,
    required this.scrollController,
  });

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        // Header
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 8),
          decoration: const BoxDecoration(
            color: Colors.black, // Strict Black
            border: Border(bottom: BorderSide(color: Colors.white, width: 1)),
          ),
          width: double.infinity,
          child: Row(
            children: [
              const Icon(Icons.terminal, color: SpookyColors.white, size: 14),
              const SizedBox(width: 8),
              Text(
                "OUTPUT",
                style: GoogleFonts.spaceMono(
                  fontWeight: FontWeight.bold,
                  color: SpookyColors.white,
                  fontSize: 11,
                  letterSpacing: 0.5,
                ),
              ),
            ],
          ),
        ),
        // Content
        Expanded(
          child: Container(
            color: Colors.black, // Pure black
            child: Scrollbar(
              // Add scrollbar for better UX
              controller: scrollController,
              child: TextField(
                controller: controller,
                scrollController: scrollController,
                readOnly: true,
                showCursor: false, // Hide blinking cursor
                mouseCursor: SystemMouseCursors.text,
                maxLines: null,
                style: GoogleFonts.spaceMono(
                  color: SpookyColors.white,
                  fontSize: 12,
                  height: 1.4,
                ),
                decoration: const InputDecoration(
                  contentPadding: EdgeInsets.all(8),
                  border: InputBorder.none,
                  isDense: true,
                  filled: true,
                  hoverColor: Colors.transparent, // Disable hover effect
                  fillColor: const Color(0xFF000000), // Force pure black
                  enabledBorder: const OutlineInputBorder(
                    borderRadius: BorderRadius.zero,
                    borderSide: BorderSide.none,
                  ),
                  focusedBorder: const OutlineInputBorder(
                    borderRadius: BorderRadius.zero,
                    borderSide: BorderSide.none,
                  ),
                ),
              ),
            ),
          ),
        ),
      ],
    );
  }
}
