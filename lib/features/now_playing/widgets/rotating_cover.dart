import 'dart:async';
import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import 'package:mofu/app/theme.dart';
import 'package:mofu/src/rust/api/player.dart';

/// 播放时匀速旋转的圆形封面；暂停时保持当前角度。
///
/// 封面只解码一次。唱盘外圈 / 环 / 轴心在未旋转坐标系绘制，只旋转标签，
/// 避免 Impeller 旋转 [Canvas.drawCircle] 时圆心被对齐到整像素而颤抖。
class RotatingCover extends StatefulWidget {
  const RotatingCover({
    super.key,
    required this.cover,
    required this.playing,
    this.pauseWhenInactive = true,
    this.size = 280,
  });

  final CoverArt? cover;
  final bool playing;

  /// When true, stop spinning while the app is not resumed.
  final bool pauseWhenInactive;
  final double size;

  /// 一周旋转时长。
  static const period = Duration(seconds: 24);

  @override
  State<RotatingCover> createState() => _RotatingCoverState();
}

class _RotatingCoverState extends State<RotatingCover>
    with WidgetsBindingObserver, SingleTickerProviderStateMixin {
  late final AnimationController _turns;
  bool _appActive = true;
  bool _tickerEnabled = true;
  DateTime? _backgroundedAt;
  double _turnsAtBackground = 0;
  final GlobalKey _discKey = GlobalKey();
  Offset _pixelSnap = Offset.zero;

  double _dpr = 1;
  ui.Image? _coverImage;
  Object? _readyBakeKey;
  Object? _inflightBakeKey;
  int _bakeGen = 0;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _turns = AnimationController(vsync: this, duration: RotatingCover.period);
    // 切歌会换 key 重建 State；此时不会再次收到 lifecycle 回调，需读当前状态。
    final lifecycle = WidgetsBinding.instance.lifecycleState;
    _appActive = lifecycle == null || lifecycle == AppLifecycleState.resumed;
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _dpr = MediaQuery.devicePixelRatioOf(context).clamp(1.0, 3.0);
    _tickerEnabled = TickerMode.valuesOf(context).enabled;
    _syncPlayback();
    _requestBake();
  }

  @override
  void didUpdateWidget(covariant RotatingCover oldWidget) {
    super.didUpdateWidget(oldWidget);
    _syncPlayback();
    if (oldWidget.cover != widget.cover || oldWidget.size != widget.size) {
      _requestBake();
    }
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    final resumed = state == AppLifecycleState.resumed;
    if (_appActive && !resumed) {
      _backgroundedAt = DateTime.now();
      _turnsAtBackground = _turns.value;
    } else if (!_appActive && resumed) {
      final started = _backgroundedAt;
      if (started != null && !widget.pauseWhenInactive && widget.playing) {
        final elapsed = DateTime.now().difference(started);
        _turns.value =
            (_turnsAtBackground +
                elapsed.inMicroseconds / RotatingCover.period.inMicroseconds) %
            1.0;
      }
      _backgroundedAt = null;
    }
    _appActive = resumed;
    _syncPlayback();
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _bakeGen++;
    _turns.dispose();
    _coverImage?.dispose();
    _coverImage = null;
    super.dispose();
  }

  bool get _shouldSpin {
    final activeOk = !widget.pauseWhenInactive || _appActive;
    return widget.playing && activeOk && _tickerEnabled;
  }

  void _syncPixelSnap() {
    final box = _discKey.currentContext?.findRenderObject() as RenderBox?;
    if (box == null || !box.hasSize) {
      return;
    }
    final dpr = MediaQuery.devicePixelRatioOf(context);
    final origin = box.localToGlobal(Offset.zero);
    final next = Offset(
      (origin.dx * dpr).round() / dpr - origin.dx,
      (origin.dy * dpr).round() / dpr - origin.dy,
    );
    if ((next - _pixelSnap).distanceSquared > 1e-8) {
      setState(() => _pixelSnap = next);
    }
  }

  void _syncPlayback() {
    if (_shouldSpin) {
      if (!_turns.isAnimating) {
        _turns.repeat();
      }
    } else if (_turns.isAnimating) {
      _turns.stop();
    }
  }

  Object _bakeKey() => (widget.size, _dpr, identityHashCode(widget.cover));

  void _requestBake() {
    final key = _bakeKey();
    if (key == _readyBakeKey) {
      return;
    }
    if (key == _inflightBakeKey) {
      return;
    }
    _inflightBakeKey = key;
    final gen = ++_bakeGen;
    unawaited(_bake(gen: gen, key: key));
  }

  Future<void> _bake({required int gen, required Object key}) async {
    ui.Image? coverImage;
    try {
      final pixel = (widget.size * _dpr).round().clamp(1, 4096);
      final coverPx = (pixel * 0.72).round().clamp(1, 4096);
      coverImage = await _decodeCover(widget.cover, coverPx);
    } catch (_) {
      coverImage?.dispose();
      coverImage = null;
    }

    if (!mounted || gen != _bakeGen) {
      coverImage?.dispose();
      return;
    }
    if (_inflightBakeKey == key) {
      _inflightBakeKey = null;
    }

    final previous = _coverImage;
    setState(() {
      _coverImage = coverImage;
      _readyBakeKey = key;
    });
    previous?.dispose();
  }

  static Future<ui.Image?> _decodeCover(CoverArt? cover, int targetPx) async {
    final bytes = cover?.data;
    if (bytes == null || bytes.isEmpty) {
      return null;
    }
    final codec = await ui.instantiateImageCodec(
      bytes,
      targetWidth: targetPx,
      targetHeight: targetPx,
    );
    try {
      final frame = await codec.getNextFrame();
      return frame.image;
    } finally {
      codec.dispose();
    }
  }

  @override
  Widget build(BuildContext context) {
    final size = (widget.size * _dpr).round().clamp(1, 4096) / _dpr;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) {
        _syncPixelSnap();
      }
    });

    return SizedBox(
      key: _discKey,
      width: size,
      height: size,
      child: Transform.translate(
        offset: _pixelSnap,
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
            CustomPaint(
              size: Size.square(size),
              painter: _VinylPainter(dpr: _dpr),
            ),
            ListenableBuilder(
              listenable: _turns,
              builder: (context, _) {
                return CustomPaint(
                  size: Size.square(size),
                  painter: RotatingCoverPainter(
                    turns: _turns.value,
                    cover: _coverImage,
                    dpr: _dpr,
                  ),
                );
              },
            ),
            CustomPaint(size: Size.square(size), painter: _PinPainter(dpr: _dpr)),
          ],
        ),
      ),
    );
  }
}

/// 外圈不转，只转封面标签。
@visibleForTesting
class RotatingCoverPainter extends CustomPainter {
  RotatingCoverPainter({
    required this.turns,
    required this.cover,
    required this.dpr,
  });

  final double turns;
  final ui.Image? cover;
  final double dpr;

  @override
  void paint(Canvas canvas, Size size) {
    _DiscBaker.paintLabel(
      canvas,
      pixelSize: size.width,
      cover: cover,
      turns: turns,
    );
  }

  @override
  bool shouldRepaint(covariant RotatingCoverPainter oldDelegate) {
    return oldDelegate.turns != turns ||
        oldDelegate.cover != cover ||
        oldDelegate.dpr != dpr;
  }
}

/// 唱盘矢量绘制。只使用 [Canvas.drawCircle]；外圈不随 [turns] 旋转。
class _DiscBaker {
  const _DiscBaker._();

  static void paintVinyl(
    Canvas canvas, {
    required double pixelSize,
    required double dpr,
  }) {
    final center = Offset(pixelSize / 2, pixelSize / 2);
    final radius = pixelSize / 2;
    canvas.drawCircle(
      center,
      radius,
      Paint()
        ..shader = ui.Gradient.radial(
          center,
          radius,
          [
            StrawberryCream.wine.withValues(alpha: 0.85),
            StrawberryCream.wine,
            const Color(0xFF2A1018),
          ],
          const [0.0, 0.55, 1.0],
        ),
    );
    for (final ratio in [0.92, 0.78, 0.64]) {
      canvas.drawCircle(
        center,
        radius * ratio,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = dpr
          ..color = Colors.white.withValues(alpha: 0.06),
      );
    }
  }

  static void paintPin(
    Canvas canvas, {
    required double pixelSize,
    required double dpr,
  }) {
    final center = Offset(pixelSize / 2, pixelSize / 2);
    final pinRadius = pixelSize / 2 * 0.12;
    canvas.drawCircle(center, pinRadius, Paint()..color = StrawberryCream.cream);
    canvas.drawCircle(
      center,
      pinRadius,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2 * dpr
        ..color = StrawberryCream.strawberry.withValues(alpha: 0.5),
    );
  }

  static void paintLabel(
    Canvas canvas, {
    required double pixelSize,
    required ui.Image? cover,
    required double turns,
  }) {
    final center = Offset(pixelSize / 2, pixelSize / 2);
    final radius = pixelSize / 2;
    final coverRadius = radius * 0.72;
    final coverRect = Rect.fromCircle(center: center, radius: coverRadius);
    final angle = turns * math.pi * 2;
    if (cover != null) {
      _paintCoverImage(canvas, cover: cover, dest: coverRect, angle: angle);
    } else {
      final placeholder = LinearGradient(
        begin: Alignment.topLeft,
        end: Alignment.bottomRight,
        colors: StrawberryCream.coverPlaceholder.colors,
        transform: GradientRotation(angle),
      );
      canvas.drawCircle(
        center,
        coverRadius,
        Paint()..shader = placeholder.createShader(coverRect),
      );
      canvas.save();
      canvas.translate(center.dx, center.dy);
      canvas.rotate(angle);
      canvas.translate(-center.dx, -center.dy);
      _paintMusicNote(canvas, center: center, iconSize: coverRadius * 2 * 0.38);
      canvas.restore();
    }
  }

  static void _paintCoverImage(
    Canvas canvas, {
    required ui.Image cover,
    required Rect dest,
    required double angle,
  }) {
    // ImageShader 的矩阵与 Canvas.transform 相同：把图像坐标变到画布。
    // 先按 dest 缩小，再绕 dest 圆心旋转，保证整张封面落入标签圆并绕轴心转。
    final matrix = Matrix4.identity()
      ..translateByDouble(dest.center.dx, dest.center.dy, 0, 1)
      ..rotateZ(angle)
      ..translateByDouble(-dest.width / 2, -dest.height / 2, 0, 1)
      ..scaleByDouble(
        dest.width / cover.width,
        dest.height / cover.height,
        1,
        1,
      );
    final shader = ui.ImageShader(
      cover,
      TileMode.clamp,
      TileMode.clamp,
      matrix.storage,
      filterQuality: FilterQuality.medium,
    );
    try {
      canvas.drawCircle(
        dest.center,
        dest.shortestSide / 2,
        Paint()..shader = shader,
      );
    } finally {
      shader.dispose();
    }
  }

  static void _paintMusicNote(
    Canvas canvas, {
    required Offset center,
    required double iconSize,
  }) {
    const icon = Icons.music_note_rounded;
    final painter = TextPainter(textDirection: TextDirection.ltr)
      ..text = TextSpan(
        text: String.fromCharCode(icon.codePoint),
        style: TextStyle(
          fontSize: iconSize,
          fontFamily: icon.fontFamily,
          package: icon.fontPackage,
          color: StrawberryCream.wine.withValues(alpha: 0.45),
        ),
      );
    painter.layout();
    painter.paint(
      canvas,
      Offset(center.dx - painter.width / 2, center.dy - painter.height / 2),
    );
  }
}

class _VinylPainter extends CustomPainter {
  _VinylPainter({required this.dpr});

  final double dpr;

  @override
  void paint(Canvas canvas, Size size) {
    _DiscBaker.paintVinyl(canvas, pixelSize: size.width, dpr: dpr);
  }

  @override
  bool shouldRepaint(covariant _VinylPainter oldDelegate) {
    return oldDelegate.dpr != dpr;
  }
}

class _PinPainter extends CustomPainter {
  _PinPainter({required this.dpr});

  final double dpr;

  @override
  void paint(Canvas canvas, Size size) {
    _DiscBaker.paintPin(canvas, pixelSize: size.width, dpr: dpr);
  }

  @override
  bool shouldRepaint(covariant _PinPainter oldDelegate) {
    return oldDelegate.dpr != dpr;
  }
}
