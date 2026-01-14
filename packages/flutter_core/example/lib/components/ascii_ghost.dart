import 'dart:async';
import 'dart:math';
import 'package:flutter/material.dart';
import 'package:flutter_core_example/core/theme.dart';
import 'package:google_fonts/google_fonts.dart';

class AsciiGhost extends StatefulWidget {
  final Color color;
  const AsciiGhost({super.key, this.color = SpookyColors.white});

  @override
  State<AsciiGhost> createState() => _AsciiGhostState();
}

class _AsciiGhostState extends State<AsciiGhost> {
  // ASCII Data
  static const List<String> _rawGhost = [
    "   ▄▄████████▄▄   ", // 0
    " ▄██████████████▄ ", // 1
    " ████████████████ ", // 2
    " ████  ████  ████ ", // 3 (Eyes)
    " ████████████████ ", // 4
    " ██████▀  ▀██████ ", // 5
    " ██████    ██████ ", // 6
    " ██████▄  ▄██████ ", // 7
    " ████████████████ ", // 8
    " ████████████████ ", // 9
    " ██▄ ▀█▄▀ █▀ ▄█▄█ ", // 10
    "  ▀   █   █   ▀ ▀ ", // 11
  ];

  static const String _eyesClosed =
      " ████▀▀████▀▀████ "; // Replacement for row 3

  double _phase = 0.0;
  bool _blink = false;
  Timer? _animTimer;
  Timer? _blinkResetTimer;

  @override
  void initState() {
    super.initState();
    _startAnimation();
  }

  @override
  void dispose() {
    _animTimer?.cancel();
    _blinkResetTimer?.cancel();
    super.dispose();
  }

  void _startAnimation() {
    _animTimer = Timer.periodic(const Duration(milliseconds: 100), (timer) {
      if (!mounted) return;
      setState(() {
        // Increment phase for the wave
        _phase += 0.5;

        // Random blink (approx 5% chance per tick)
        if (Random().nextDouble() > 0.95 && !_blink) {
          _blink = true;
          _blinkResetTimer?.cancel();
          _blinkResetTimer = Timer(const Duration(milliseconds: 200), () {
            if (mounted) setState(() => _blink = false);
          });
        }
      });
    });
  }

  // Helper to pad lines for movement
  // Matches React Logic:
  // if (dir < -0.7) return trimmed + "  "; // Left
  // if (dir > 0.7) return "  " + trimmed; // Right
  // return " " + trimmed + " "; // Center
  String _shiftLine(String line, double dir) {
    final trimmed = line.trim();
    if (dir < -0.7) return "$trimmed  "; // Left
    if (dir > 0.7) return "  $trimmed"; // Right
    return " $trimmed "; // Center
  }

  @override
  Widget build(BuildContext context) {
    final buffer = StringBuffer();

    for (int i = 0; i < _rawGhost.length; i++) {
      // Calculate sine wave offset based on row index and time phase
      // Frequency = 0.6
      final waveValue = sin(i * 0.6 + _phase);

      // Use the eyes-closed row if blinking and on index 3
      final content = (_blink && i == 3) ? _eyesClosed : _rawGhost[i];

      final shiftedLine = _shiftLine(content, waveValue);
      buffer.writeln(shiftedLine);
    }

    return Text(
      buffer.toString(),
      style: GoogleFonts.firaCode(
        fontSize: 10, // Small text for ASCII
        fontWeight: FontWeight.bold,
        height: 1.0, // Tight line height
        color: widget.color,
        letterSpacing: -1.0, // Tight letter spacing often helps ASCII art
      ),
      textAlign: TextAlign.center,
    );
  }
}
