import 'package:flutter/material.dart';
import 'package:yaru/theme.dart';

/// Mofu 色板；控件造型由 Yaru 主题提供。
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

  /// Yaru 控件主题 + 草莓奶油色板。
  static ThemeData theme() {
    final yaru = createYaruLightTheme(
      primaryColor: strawberry,
      lightBaseColor: cream,
      darkBaseColor: wine,
      elevatedButtonColor: strawberry,
      elevatedButtonTextColor: Colors.white,
      errorColor: const Color(0xFFB3261E),
    );

    return yaru.copyWith(
      scaffoldBackgroundColor: Colors.white,
      colorScheme: yaru.colorScheme.copyWith(
        secondary: strawberrySoft,
        onSecondary: wine,
      ),
      appBarTheme: yaru.appBarTheme.copyWith(
        backgroundColor: Colors.transparent,
        elevation: 0,
        scrolledUnderElevation: 0,
        foregroundColor: wine,
        centerTitle: false,
      ),
      cardTheme: yaru.cardTheme.copyWith(
        color: card,
        elevation: 0,
        margin: EdgeInsets.zero,
        shadowColor: strawberry.withValues(alpha: 0.18),
      ),
      listTileTheme: yaru.listTileTheme.copyWith(
        selectedTileColor: strawberryMist,
        selectedColor: strawberry,
        iconColor: wineMuted,
        textColor: wine,
      ),
      snackBarTheme: yaru.snackBarTheme.copyWith(
        backgroundColor: wine,
        contentTextStyle: const TextStyle(color: cream),
        behavior: SnackBarBehavior.floating,
      ),
    );
  }

  /// 无封面时的粉白占位渐变。
  static const LinearGradient coverPlaceholder = LinearGradient(
    begin: Alignment.topLeft,
    end: Alignment.bottomRight,
    colors: [Color(0xFFFFE0E8), Color(0xFFFFF7F2), Color(0xFFF4A0B0)],
  );

  static List<BoxShadow> softShadow({double blur = 24, double dy = 10}) => [
    BoxShadow(
      color: strawberry.withValues(alpha: 0.16),
      blurRadius: blur,
      offset: Offset(0, dy),
    ),
  ];
}
