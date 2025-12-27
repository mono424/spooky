import 'package:flutter/material.dart';
import 'package:google_fonts/google_fonts.dart';

class SpookyColors {
  static const primary = Color(0xFFE94560);
  static const secondary = Color(0xFF533483);
  static const background = Color(0xFF0a0e1a); // Deep Navy
  static const surface = Color(0xFF0f1520); // Light Deep Navy
  static const accent = Color(0xFF0F3460); // Navy

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
      colorScheme: ColorScheme.dark(
        primary: SpookyColors.primary,
        secondary: SpookyColors.secondary,
        surface: SpookyColors.surface,
        background: SpookyColors.background,
        onBackground: SpookyColors.white,
        onSurface: SpookyColors.white,
      ),
      textTheme: GoogleFonts.interTextTheme(
        ThemeData.dark().textTheme.apply(
          bodyColor: SpookyColors.white,
          displayColor: SpookyColors.white,
        ),
      ),
      elevatedButtonTheme: ElevatedButtonThemeData(
        style: ElevatedButton.styleFrom(
          backgroundColor: SpookyColors.primary,
          foregroundColor: SpookyColors.white,
          textStyle: GoogleFonts.inter(
            fontWeight: FontWeight.w600,
            fontSize: 16,
          ),
          padding: const EdgeInsets.symmetric(horizontal: 24, vertical: 16),
          shape: RoundedRectangleBorder(borderRadius: BorderRadius.circular(4)),
          elevation: 0,
        ),
      ),
      inputDecorationTheme: InputDecorationTheme(
        floatingLabelBehavior:
            FloatingLabelBehavior.never, // Label doesn't float
        prefixIconColor: const _SpookyInputIconColor(), // Reactive icon color
        filled: true,
        fillColor: SpookyColors.white10,
        hintStyle: GoogleFonts.inter(color: SpookyColors.white60),
        labelStyle: GoogleFonts.inter(color: SpookyColors.white60),
        border: OutlineInputBorder(
          borderRadius: BorderRadius.circular(4),
          borderSide: BorderSide.none,
        ),
        enabledBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(4),
          borderSide: BorderSide(color: SpookyColors.white10, width: 1),
        ),
        focusedBorder: OutlineInputBorder(
          borderRadius: BorderRadius.circular(4),
          borderSide: BorderSide(color: SpookyColors.primary, width: 2),
        ),
        contentPadding: const EdgeInsets.symmetric(
          horizontal: 16,
          vertical: 16,
        ),
      ),
    );
  }
}

class _SpookyInputIconColor extends MaterialStateColor {
  const _SpookyInputIconColor() : super(0x99FFFFFF); // Default to white60 value

  @override
  Color resolve(Set<MaterialState> states) {
    if (states.contains(MaterialState.focused)) {
      return SpookyColors.primary;
    }
    return SpookyColors.white60;
  }
}
