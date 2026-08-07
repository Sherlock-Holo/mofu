import 'package:flutter/material.dart';
import 'package:flutter/scheduler.dart';

import 'package:mofu/app/theme.dart';
import 'package:mofu/src/rust/api/player.dart';
import 'package:mofu/widgets/cover_view.dart';

/// 播放时匀速旋转的圆形封面；暂停时保持当前角度。
///
/// 唱片面在 [RepaintBoundary] 内只栅格化一次；旋转以约 30fps 更新矩阵，
/// 黑胶观感仍流畅，但合成次数约为 60fps 的一半。应用退到后台时停止 ticker。
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

  /// 一周旋转时长。
  static const period = Duration(seconds: 24);

  /// 旋转刷新间隔（约 30fps）。
  static const frameInterval = Duration(milliseconds: 33);

  @override
  State<RotatingCover> createState() => _RotatingCoverState();
}

class _RotatingCoverState extends State<RotatingCover>
    with SingleTickerProviderStateMixin, WidgetsBindingObserver {
  late final Ticker _ticker;
  double _turns = 0;
  Duration _lastElapsed = Duration.zero;
  Duration _sincePaint = Duration.zero;
  bool _appActive = true;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _ticker = createTicker(_onTick);
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _syncPlayback();
  }

  @override
  void didUpdateWidget(covariant RotatingCover oldWidget) {
    super.didUpdateWidget(oldWidget);
    _syncPlayback();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    _appActive = state == AppLifecycleState.resumed;
    _syncPlayback();
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _ticker.dispose();
    super.dispose();
  }

  bool get _shouldSpin =>
      widget.playing && _appActive && TickerMode.of(context);

  void _syncPlayback() {
    if (_shouldSpin) {
      if (!_ticker.isActive) {
        _lastElapsed = Duration.zero;
        _sincePaint = Duration.zero;
        _ticker.start();
      }
    } else if (_ticker.isActive) {
      _ticker.stop();
    }
  }

  void _onTick(Duration elapsed) {
    final delta = elapsed - _lastElapsed;
    _lastElapsed = elapsed;
    if (!_shouldSpin || delta <= Duration.zero) {
      return;
    }

    _sincePaint += delta;
    if (_sincePaint < RotatingCover.frameInterval) {
      return;
    }
    final step = _sincePaint;
    _sincePaint = Duration.zero;
    setState(() {
      _turns =
          (_turns + step.inMicroseconds / RotatingCover.period.inMicroseconds) %
          1.0;
    });
  }

  @override
  Widget build(BuildContext context) {
    final size = widget.size;
    return SizedBox(
      width: size,
      height: size,
      child: Stack(
        alignment: Alignment.center,
        children: [
          RepaintBoundary(
            child: IgnorePointer(
              child: Container(
                width: size * 0.92,
                height: size * 0.92,
                decoration: BoxDecoration(
                  shape: BoxShape.circle,
                  boxShadow: StrawberryCream.softShadow(blur: 12, dy: 8),
                ),
              ),
            ),
          ),
          Transform.rotate(
            angle: _turns * 6.283185307179586,
            child: RepaintBoundary(
              child: _DiscFace(cover: widget.cover, size: size),
            ),
          ),
        ],
      ),
    );
  }
}

class _DiscFace extends StatelessWidget {
  const _DiscFace({required this.cover, required this.size});

  final CoverArt? cover;
  final double size;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: size,
      height: size,
      child: Stack(
        alignment: Alignment.center,
        children: [
          DecoratedBox(
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
            child: SizedBox(width: size, height: size),
          ),
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
          CoverView(cover: cover, size: size * 0.72, circular: true),
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
    );
  }
}
