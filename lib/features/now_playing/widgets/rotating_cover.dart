import 'package:flutter/material.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/widgets/cover_view.dart';
import 'package:mofu/src/rust/api/player.dart';

/// 播放时匀速旋转的圆形封面；暂停时保持当前角度。
class RotatingCover extends StatefulWidget {
  const RotatingCover({
    super.key,
    required this.cover,
    required this.playing,
    this.size = 280,
  });

  final CoverArt? cover;
  final bool playing;
  final double size;

  @override
  State<RotatingCover> createState() => _RotatingCoverState();
}

class _RotatingCoverState extends State<RotatingCover>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller;

  @override
  void initState() {
    super.initState();
    _controller = AnimationController(
      vsync: this,
      duration: const Duration(seconds: 24),
    );
    if (widget.playing) {
      _controller.repeat();
    }
  }

  @override
  void didUpdateWidget(covariant RotatingCover oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.playing && !_controller.isAnimating) {
      _controller.repeat();
    } else if (!widget.playing && _controller.isAnimating) {
      _controller.stop();
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final size = widget.size;
    return AnimatedBuilder(
      animation: _controller,
      builder: (context, child) {
        return Transform.rotate(
          angle: _controller.value * 6.283185307179586,
          child: child,
        );
      },
      child: Container(
        width: size,
        height: size,
        decoration: BoxDecoration(
          shape: BoxShape.circle,
          boxShadow: StrawberryCream.softShadow(blur: 28, dy: 14),
        ),
        child: Stack(
          alignment: Alignment.center,
          children: [
            // 外圈唱片纹理
            Container(
              width: size,
              height: size,
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                gradient: RadialGradient(
                  colors: [
                    StrawberryCream.wine.withValues(alpha: 0.85),
                    StrawberryCream.wine,
                    const Color(0xFF2A1018),
                  ],
                  stops: const [0.0, 0.55, 1.0],
                ),
              ),
            ),
            // 细环
            for (final ratio in [0.92, 0.78, 0.64])
              Container(
                width: size * ratio,
                height: size * ratio,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  border: Border.all(
                    color: Colors.white.withValues(alpha: 0.06),
                    width: 1,
                  ),
                ),
              ),
            CoverView(
              cover: widget.cover,
              size: size * 0.72,
              circular: true,
            ),
            Container(
              width: size * 0.12,
              height: size * 0.12,
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                color: StrawberryCream.cream,
                border: Border.all(
                  color: StrawberryCream.strawberry.withValues(alpha: 0.5),
                  width: 2,
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}
