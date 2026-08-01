import 'package:flutter/material.dart';

/// Mofu色板与 Material 3 主题。
abstract final class StrawberryCream {
  static const Color cream = Color(0xFFFFF7F2);
  static const Color creamDeep = Color(0xFFFFEDE3);
  static const Color strawberry = Color(0xFFE85A7A);
  static const Color strawberrySoft = Color(0xFFF4A0B0);
  static const Color strawberryMist = Color(0x33E85A7A);
  static const Color wine = Color(0xFF5C2A3A);
  static const Color wineMuted = Color(0xFF8A5A68);
  static const Color card = Color(0xCCFFFFFF);
  static const Color cardSolid = Color(0xFFFAFAFA);
  static const double radius = 16;
  static const double radiusSm = 10;

  static ThemeData theme() {
    final base = ColorScheme.fromSeed(
      seedColor: strawberry,
      brightness: Brightness.light,
      primary: strawberry,
      onPrimary: Colors.white,
      secondary: strawberrySoft,
      onSecondary: wine,
      surface: cream,
      onSurface: wine,
      error: const Color(0xFFB3261E),
    );

    final shape = RoundedRectangleBorder(
      borderRadius: BorderRadius.circular(radius),
    );

    return ThemeData(
      useMaterial3: true,
      colorScheme: base,
      scaffoldBackgroundColor: cream,
      textTheme: const TextTheme(
        displayLarge: TextStyle(
          color: wine,
          fontWeight: FontWeight.w700,
          letterSpacing: -0.5,
        ),
        headlineMedium: TextStyle(
          color: wine,
          fontWeight: FontWeight.w700,
        ),
        titleLarge: TextStyle(
          color: wine,
          fontWeight: FontWeight.w600,
        ),
        titleMedium: TextStyle(
          color: wine,
          fontWeight: FontWeight.w600,
        ),
        bodyLarge: TextStyle(color: wine, height: 1.35),
        bodyMedium: TextStyle(color: wineMuted, height: 1.35),
        labelLarge: TextStyle(
          color: wine,
          fontWeight: FontWeight.w600,
        ),
      ),
      appBarTheme: const AppBarTheme(
        backgroundColor: Colors.transparent,
        elevation: 0,
        scrolledUnderElevation: 0,
        foregroundColor: wine,
        centerTitle: false,
      ),
      cardTheme: CardThemeData(
        color: card,
        elevation: 0,
        shape: shape,
        margin: EdgeInsets.zero,
        shadowColor: strawberry.withValues(alpha: 0.18),
      ),
      dialogTheme: DialogThemeData(
        backgroundColor: cream,
        shape: shape,
      ),
      filledButtonTheme: FilledButtonThemeData(
        style: FilledButton.styleFrom(
          backgroundColor: strawberry,
          foregroundColor: Colors.white,
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(radiusSm),
          ),
          padding: const EdgeInsets.symmetric(horizontal: 18, vertical: 12),
        ),
      ),
      outlinedButtonTheme: OutlinedButtonThemeData(
        style: OutlinedButton.styleFrom(
          foregroundColor: wine,
          side: BorderSide(color: strawberry.withValues(alpha: 0.35)),
          shape: RoundedRectangleBorder(
            borderRadius: BorderRadius.circular(radiusSm),
          ),
        ),
      ),
      iconButtonTheme: IconButtonThemeData(
        style: IconButton.styleFrom(foregroundColor: wine),
      ),
      sliderTheme: SliderThemeData(
        activeTrackColor: strawberry,
        inactiveTrackColor: strawberryMist,
        thumbColor: strawberry,
        overlayColor: strawberry.withValues(alpha: 0.16),
        trackHeight: 3,
      ),
      listTileTheme: ListTileThemeData(
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(radiusSm),
        ),
        selectedTileColor: strawberryMist,
        iconColor: wineMuted,
        textColor: wine,
      ),
      dividerColor: strawberry.withValues(alpha: 0.12),
      snackBarTheme: SnackBarThemeData(
        backgroundColor: wine,
        contentTextStyle: const TextStyle(color: cream),
        behavior: SnackBarBehavior.floating,
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(radiusSm),
        ),
      ),
    );
  }

  /// 无封面时的粉白占位渐变。
  static const LinearGradient coverPlaceholder = LinearGradient(
    begin: Alignment.topLeft,
    end: Alignment.bottomRight,
    colors: [
      Color(0xFFFFE0E8),
      Color(0xFFFFF7F2),
      Color(0xFFF4A0B0),
    ],
  );

  static List<BoxShadow> softShadow({double blur = 24, double dy = 10}) => [
        BoxShadow(
          color: strawberry.withValues(alpha: 0.16),
          blurRadius: blur,
          offset: Offset(0, dy),
        ),
      ];
}
