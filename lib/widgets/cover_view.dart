import 'dart:typed_data';
import 'dart:ui' show ImageFilter;

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
    final radius =
        borderRadius ??
        BorderRadius.circular(circular ? size / 2 : StrawberryCream.radiusSm);
    final image = _bytes;
    final cachePx = (size * MediaQuery.devicePixelRatioOf(context))
        .round()
        .clamp(1, 4096);

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
    return _CoverPlaceholder(size: size);
  }
}

class _CoverPlaceholder extends StatelessWidget {
  const _CoverPlaceholder({required this.size});

  final double size;

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      decoration: const BoxDecoration(
        gradient: StrawberryCream.coverPlaceholder,
      ),
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

/// 磨砂玻璃封面背景 + 粉白遮罩；无封面时用同色系渐变。
///
/// 背景层按曲目缓存，不随封面旋转重绘，因此可以用 [ImageFiltered]
/// 做一次全屏高斯模糊。
class BlurCoverBackground extends StatelessWidget {
  const BlurCoverBackground({super.key, required this.cover, this.child});

  final CoverArt? cover;
  final Widget? child;

  @override
  Widget build(BuildContext context) {
    final bytes = cover?.data;
    final hasCover = bytes != null && bytes.isNotEmpty;
    final mediaSize = MediaQuery.sizeOf(context);
    final dpr = MediaQuery.devicePixelRatioOf(context).clamp(1.0, 2.0);
    // 低分辨率解码降低模糊成本；只约束一边以保留整张封面。
    final cachePx = (mediaSize.longestSide * dpr / 16).round().clamp(48, 128);

    return Stack(
      fit: StackFit.expand,
      children: [
        RepaintBoundary(
          child: hasCover
              ? ClipRect(
                  child: ImageFiltered(
                    imageFilter: ImageFilter.blur(
                      sigmaX: 48,
                      sigmaY: 48,
                      tileMode: TileMode.clamp,
                    ),
                    child: Image.memory(
                      bytes,
                      width: mediaSize.width,
                      height: mediaSize.height,
                      fit: BoxFit.fill,
                      cacheWidth: cachePx,
                      gaplessPlayback: true,
                      filterQuality: FilterQuality.medium,
                      errorBuilder: (_, _, _) =>
                          const ColoredBox(color: StrawberryCream.creamDeep),
                    ),
                  ),
                )
              : const DecoratedBox(
                  decoration: BoxDecoration(
                    gradient: StrawberryCream.coverPlaceholder,
                  ),
                ),
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
