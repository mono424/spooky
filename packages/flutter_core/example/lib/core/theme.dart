import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';

class SpookyColors {
  static const primary = Colors.white;
  static const secondary = Color(0xFF1a1a1a); // Dark Grey
  static const background = Colors.black;
  static const surface = Colors.black;
  static const accent = Colors.white;

  static const white = Colors.white;
  static const white60 = Color(0x99FFFFFF);
  static const white10 = Color(0x1AFFFFFF);

  static const transparent = Colors.transparent;
}

class SpookyTheme {
  static ThemeData get theme {
    return ThemeData(
      useMaterial3: true,
      scaffoldBackgroundColor: SpookyColors.background,
      colorScheme: const ColorScheme.dark(
        primary: SpookyColors.primary,
        secondary: SpookyColors.secondary,
        surface: SpookyColors.surface,
        background: SpookyColors.background,
        onBackground: SpookyColors.white,
        onSurface: SpookyColors.white,
        error: Colors.redAccent,
        surfaceTint: Colors.transparent, // Fixes brownish tint
      ),
      textTheme: GoogleFonts.spaceMonoTextTheme(
        ThemeData.dark().textTheme.apply(
          bodyColor: SpookyColors.white,
          displayColor: SpookyColors.white,
        ),
      ),
      // Sharp, high-contrast buttons
      // Sharp, high-contrast buttons (Wireframe Style)
      // Sharp, high-contrast buttons (Wireframe Style)
      elevatedButtonTheme: ElevatedButtonThemeData(
        style: ButtonStyle(
          backgroundColor: MaterialStateProperty.resolveWith((states) {
            if (states.contains(MaterialState.hovered) ||
                states.contains(MaterialState.pressed)) {
              return SpookyColors.white; // Invert to White
            }
            return SpookyColors.background; // Default Black
          }),
          foregroundColor: MaterialStateProperty.resolveWith((states) {
            if (states.contains(MaterialState.hovered) ||
                states.contains(MaterialState.pressed)) {
              return SpookyColors.background; // Invert to Black
            }
            return SpookyColors.white; // Default White
          }),
          textStyle: MaterialStateProperty.all(
            GoogleFonts.spaceMono(fontWeight: FontWeight.w700, fontSize: 14),
          ),
          padding: MaterialStateProperty.all(
            const EdgeInsets.symmetric(horizontal: 24, vertical: 20),
          ),
          shape: MaterialStateProperty.all(
            const RoundedRectangleBorder(
              borderRadius: BorderRadius.zero,
              side: BorderSide(color: SpookyColors.white),
            ),
          ),
          elevation: MaterialStateProperty.all(0),
          overlayColor: MaterialStateProperty.all(Colors.transparent),
        ),
      ),
      textButtonTheme: TextButtonThemeData(
        style:
            TextButton.styleFrom(
              foregroundColor: SpookyColors.white,
              shape: const RoundedRectangleBorder(
                borderRadius: BorderRadius.zero,
              ),
            ).copyWith(
              overlayColor: MaterialStateProperty.resolveWith((states) {
                if (states.contains(MaterialState.hovered)) {
                  return SpookyColors.white.withOpacity(0.1);
                }
                if (states.contains(MaterialState.pressed)) {
                  return SpookyColors.white.withOpacity(0.2);
                }
                return null;
              }),
            ),
      ),
      // Terminal-style inputs
      inputDecorationTheme: InputDecorationTheme(
        floatingLabelBehavior: FloatingLabelBehavior.always,
        filled: true,
        fillColor: SpookyColors.surface, // Force black simple color
        hintStyle: GoogleFonts.spaceMono(
          color: SpookyColors.white60,
          fontSize: 12,
        ),
        labelStyle: GoogleFonts.spaceMono(
          color: SpookyColors.white,
          fontWeight: FontWeight.bold,
        ),
        // Default Border (White Outline)
        border: const OutlineInputBorder(
          borderRadius: BorderRadius.zero,
          borderSide: BorderSide(color: SpookyColors.white, width: 1),
        ),
        enabledBorder: const OutlineInputBorder(
          borderRadius: BorderRadius.zero,
          borderSide: BorderSide(color: SpookyColors.white, width: 1),
        ),
        focusedBorder: const OutlineInputBorder(
          borderRadius: BorderRadius.zero,
          borderSide: BorderSide(color: SpookyColors.white, width: 2),
        ),
        errorBorder: const OutlineInputBorder(
          borderRadius: BorderRadius.zero,
          borderSide: BorderSide(color: Colors.redAccent, width: 1),
        ),
        contentPadding: const EdgeInsets.symmetric(
          horizontal: 16,
          vertical: 20,
        ),
      ),
      dividerTheme: const DividerThemeData(
        color: SpookyColors.white,
        thickness: 1,
        space: 1,
      ),
      scrollbarTheme: ScrollbarThemeData(
        thumbColor: MaterialStateProperty.all(Colors.transparent),
        trackColor: MaterialStateProperty.all(Colors.transparent),
        trackBorderColor: MaterialStateProperty.all(Colors.transparent),
        thickness: MaterialStateProperty.all(0),
        thumbVisibility: MaterialStateProperty.all(false),
        trackVisibility: MaterialStateProperty.all(false),
      ),
    );
  }
}
