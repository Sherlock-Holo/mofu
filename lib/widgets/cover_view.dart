import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/src/rust/api/player.dart';

/// 封面展示：有图显示字节图，无图用粉白渐变占位。
class CoverView extends StatelessWidget {
  const CoverView({
    super.key,
    this.cover,
    this.size = 64,
    this.borderRadius,
    this.circular = false,
    this.fit = BoxFit.cover,
    this.shadow = false,
  });

  final CoverArt? cover;
  final double size;
  final BorderRadius? borderRadius;
  final bool circular;
  final BoxFit fit;
  final bool shadow;

  Uint8List? get _bytes {
    final data = cover?.data;
    if (data == null || data.isEmpty) {
      return null;
    }
    return data;
  }

  @override
  Widget build(BuildContext context) {
    final radius = borderRadius ??
        BorderRadius.circular(
          circular ? size / 2 : StrawberryCream.radiusSm,
        );
    final image = _bytes;
    final cachePx =
        (size * MediaQuery.devicePixelRatioOf(context)).round().clamp(1, 4096);

    Widget child;
    if (image != null) {
      child = Image.memory(
        image,
        width: size,
        height: size,
        fit: fit,
        cacheWidth: cachePx,
        cacheHeight: cachePx,
        gaplessPlayback: true,
        errorBuilder: (_, _, _) => _placeholder(size),
      );
    } else {
      child = _placeholder(size);
    }

    return Container(
      width: size,
      height: size,
      decoration: BoxDecoration(
        borderRadius: circular ? null : radius,
        shape: circular ? BoxShape.circle : BoxShape.rectangle,
        boxShadow: shadow ? StrawberryCream.softShadow(blur: 18, dy: 8) : null,
      ),
      clipBehavior: Clip.antiAlias,
      child: child,
    );
  }

  Widget _placeholder(double size) {
    return DecoratedBox(
      decoration:
          const BoxDecoration(gradient: StrawberryCream.coverPlaceholder),
      child: Center(
        child: Icon(
          Icons.music_note_rounded,
          size: size * 0.38,
          color: StrawberryCream.wine.withValues(alpha: 0.45),
        ),
      ),
    );
  }
}

/// 模糊放大封面背景 + 粉白遮罩；无封面时用同色系渐变。
class BlurCoverBackground extends StatelessWidget {
  const BlurCoverBackground({
    super.key,
    required this.cover,
    this.child,
    this.blurSigma = 28,
    this.scale = 1.25,
  });

  final CoverArt? cover;
  final Widget? child;
  final double blurSigma;
  final double scale;

  @override
  Widget build(BuildContext context) {
    final bytes = cover?.data;
    final hasCover = bytes != null && bytes.isNotEmpty;
    // 按可视区域 × scale × DPR 解码；模糊背景不需要原图像素。
    final mediaSize = MediaQuery.sizeOf(context);
    final dpr = MediaQuery.devicePixelRatioOf(context);
    final cacheWidth = (mediaSize.width * scale * dpr).round().clamp(1, 4096);
    final cacheHeight = (mediaSize.height * scale * dpr).round().clamp(1, 4096);

    return Stack(
      fit: StackFit.expand,
      children: [
        if (hasCover)
          Transform.scale(
            scale: scale,
            child: ImageFiltered(
              imageFilter: ui.ImageFilter.blur(
                sigmaX: blurSigma,
                sigmaY: blurSigma,
                tileMode: TileMode.clamp,
              ),
              child: Image.memory(
                bytes,
                fit: BoxFit.cover,
                cacheWidth: cacheWidth,
                cacheHeight: cacheHeight,
                gaplessPlayback: true,
                errorBuilder: (_, _, _) =>
                const ColoredBox(
                  color: StrawberryCream.creamDeep,
                ),
              ),
            ),
          )
        else
          const DecoratedBox(
            decoration:
                BoxDecoration(gradient: StrawberryCream.coverPlaceholder),
          ),
        DecoratedBox(
          decoration: BoxDecoration(
            gradient: LinearGradient(
              begin: Alignment.topCenter,
              end: Alignment.bottomCenter,
              colors: [
                StrawberryCream.cream.withValues(alpha: hasCover ? 0.55 : 0.35),
                StrawberryCream.strawberrySoft.withValues(
                  alpha: hasCover ? 0.42 : 0.28,
                ),
                StrawberryCream.cream.withValues(alpha: 0.78),
              ],
            ),
          ),
        ),
        ?child,
      ],
    );
  }
}
