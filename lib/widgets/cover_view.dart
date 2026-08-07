import 'dart:typed_data';

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

/// 低分辨率封面背景 + 粉白遮罩；无封面时用同色系渐变。
///
/// 通过低分辨率解码后双线性放大近似柔化效果，避免动画期间反复执行
/// 全屏 [ImageFiltered]。
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
    // 约 1/8 线性尺寸；放大后的平滑插值自然形成柔化背景。
    final cacheWidth = (mediaSize.width * dpr / 8).round().clamp(64, 256);
    final cacheHeight = (mediaSize.height * dpr / 8).round().clamp(64, 256);

    return Stack(
      fit: StackFit.expand,
      children: [
        RepaintBoundary(
          child: hasCover
              ? Image.memory(
                  bytes,
                  fit: BoxFit.cover,
                  cacheWidth: cacheWidth,
                  cacheHeight: cacheHeight,
                  gaplessPlayback: true,
                  filterQuality: FilterQuality.medium,
                  errorBuilder: (_, _, _) =>
                      const ColoredBox(color: StrawberryCream.creamDeep),
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
