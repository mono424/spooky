import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';
import '../../../../core/theme.dart';

class LiveQueryControls extends StatelessWidget {
  final TextEditingController tableController;
  final bool isListening;
  final VoidCallback onToggleListener;
  final VoidCallback onCreateRecord;

  const LiveQueryControls({
    super.key,
    required this.tableController,
    required this.isListening,
    required this.onToggleListener,
    required this.onCreateRecord,
  });

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(16),
      color: SpookyColors.surface,
      child: Column(
        children: [
          Row(
            children: [
              Expanded(
                child: TextField(
                  controller: tableController,
                  style: GoogleFonts.spaceMono(color: SpookyColors.white),
                  decoration: InputDecoration(
                    labelText: "Target Table",
                    labelStyle: GoogleFonts.spaceMono(
                      color: SpookyColors.white60,
                    ),
                    prefixIcon: const Icon(
                      Icons.table_chart,
                      color: SpookyColors.primary,
                    ),
                    enabledBorder: OutlineInputBorder(
                      borderRadius: BorderRadius.zero,
                      borderSide: BorderSide(
                        color: SpookyColors.white.withOpacity(0.1),
                      ),
                    ),
                    focusedBorder: const OutlineInputBorder(
                      borderRadius: BorderRadius.zero,
                      borderSide: BorderSide(color: SpookyColors.primary),
                    ),
                    isDense: true,
                    enabled: !isListening,
                    fillColor: isListening
                        ? SpookyColors.white.withOpacity(0.05)
                        : null,
                    filled: isListening,
                  ),
                ),
              ),
              const SizedBox(width: 16),
              _StatusIndicator(isListening: isListening),
            ],
          ),
          const SizedBox(height: 16),
          Row(
            children: [
              Expanded(
                child: ElevatedButton.icon(
                  onPressed: onToggleListener,
                  style: ElevatedButton.styleFrom(
                    shape: const RoundedRectangleBorder(
                      borderRadius: BorderRadius.zero,
                    ),
                    backgroundColor: isListening
                        ? Colors.redAccent.withOpacity(0.1)
                        : Colors.greenAccent.withOpacity(0.1),
                    foregroundColor: isListening
                        ? Colors.redAccent
                        : Colors.greenAccent,
                    padding: const EdgeInsets.symmetric(vertical: 16),
                    side: BorderSide(
                      color: isListening
                          ? Colors.redAccent
                          : Colors.greenAccent,
                    ),
                  ),
                  icon: Icon(isListening ? Icons.stop : Icons.play_arrow),
                  label: Text(
                    isListening ? "STOP SUBSCRIPTION" : "START SUBSCRIPTION",
                    style: GoogleFonts.spaceMono(fontWeight: FontWeight.bold),
                  ),
                ),
              ),
            ],
          ),
          const SizedBox(height: 12),
          SizedBox(
            width: double.infinity,
            child: OutlinedButton.icon(
              onPressed: onCreateRecord,
              style: OutlinedButton.styleFrom(
                shape: const RoundedRectangleBorder(
                  borderRadius: BorderRadius.zero,
                ),
                foregroundColor: SpookyColors.white,
                side: const BorderSide(color: Colors.white, width: 1.0),
                padding: const EdgeInsets.symmetric(vertical: 16),
              ),
              icon: const Icon(Icons.add_circle_outline),
              label: Text("Create Test Record", style: GoogleFonts.spaceMono()),
            ),
          ),
        ],
      ),
    );
  }
}

class _StatusIndicator extends StatelessWidget {
  final bool isListening;
  const _StatusIndicator({required this.isListening});

  @override
  Widget build(BuildContext context) {
    final color = isListening ? Colors.greenAccent : Colors.grey;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 12),
      decoration: BoxDecoration(
        color: color.withOpacity(0.1),
        borderRadius: BorderRadius.zero,
        border: Border.all(color: color.withOpacity(0.5), width: 1),
      ),
      child: Row(
        children: [
          Icon(
            isListening ? Icons.sensors : Icons.sensors_off,
            color: color,
            size: 16,
          ),
          const SizedBox(width: 8),
          Text(
            isListening ? "ACTIVE" : "IDLE",
            style: GoogleFonts.spaceMono(
              fontWeight: FontWeight.bold,
              color: color,
            ),
          ),
        ],
      ),
    );
  }
}
