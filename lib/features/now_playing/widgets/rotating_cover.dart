import 'dart:async';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';

import 'package:mofu/app/theme.dart';
import 'package:mofu/src/rust/api/player.dart';
import 'package:mofu/widgets/cover_view.dart';

/// 播放时匀速旋转的圆形封面；暂停时保持当前角度。
///
/// 唱片面用 [PictureRecorder] 离屏烘焙成位图；用 [Timer] 按约 30fps 更新角度，
/// 避免 [Ticker]/vsync 在未改角度时仍以 60Hz 唤醒帧调度。
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

  /// 旋转刷新间隔（约 30fps）。
  static const frameInterval = Duration(milliseconds: 33);

  @override
  State<RotatingCover> createState() => _RotatingCoverState();
}

class _RotatingCoverState extends State<RotatingCover>
    with WidgetsBindingObserver {
  final ValueNotifier<double> _turns = ValueNotifier(0);
  Timer? _timer;
  DateTime? _lastTick;
  bool _appActive = true;
  bool _tickerEnabled = true;

  double _dpr = 1;
  ui.Image? _baked;
  Object? _readyBakeKey;
  Object? _inflightBakeKey;
  int _bakeGen = 0;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
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
    _appActive = state == AppLifecycleState.resumed;
    _syncPlayback();
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _bakeGen++;
    _timer?.cancel();
    _timer = null;
    _turns.dispose();
    _baked?.dispose();
    _baked = null;
    super.dispose();
  }

  bool get _shouldSpin {
    final activeOk = !widget.pauseWhenInactive || _appActive;
    return widget.playing && activeOk && _tickerEnabled;
  }

  void _syncPlayback() {
    if (_shouldSpin) {
      _timer ??= Timer.periodic(RotatingCover.frameInterval, _onTimer);
    } else {
      _timer?.cancel();
      _timer = null;
      _lastTick = null;
    }
  }

  void _onTimer(Timer timer) {
    if (!mounted) {
      timer.cancel();
      return;
    }
    if (!_shouldSpin) {
      _syncPlayback();
      return;
    }
    final now = DateTime.now();
    final last = _lastTick;
    _lastTick = now;
    final step = last == null
        ? RotatingCover.frameInterval
        : now.difference(last);
    if (step <= Duration.zero) {
      return;
    }
    _turns.value =
        (_turns.value +
            step.inMicroseconds / RotatingCover.period.inMicroseconds) %
        1.0;
  }

  Object _bakeKey() => (widget.size, _dpr, identityHashCode(widget.cover));

  void _requestBake() {
    final key = _bakeKey();
    if (key == _readyBakeKey && _baked != null) {
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
    ui.Image? disc;
    try {
      final pixel = (widget.size * _dpr).round().clamp(1, 4096);
      final coverPx = (pixel * 0.72).round().clamp(1, 4096);
      coverImage = await _decodeCover(widget.cover, coverPx);
      if (!mounted || gen != _bakeGen) {
        return;
      }
      disc = await _DiscBaker.bake(
        pixelSize: pixel,
        dpr: _dpr,
        cover: coverImage,
      );
    } catch (_) {
      disc?.dispose();
      disc = null;
    } finally {
      coverImage?.dispose();
    }

    if (!mounted || gen != _bakeGen) {
      disc?.dispose();
      return;
    }
    if (_inflightBakeKey == key) {
      _inflightBakeKey = null;
    }
    if (disc == null) {
      return;
    }

    final previous = _baked;
    setState(() {
      _baked = disc;
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
    final size = widget.size;
    final baked = _baked;
    final disc = baked != null
        ? RawImage(
            image: baked,
            width: size,
            height: size,
            fit: BoxFit.fill,
            filterQuality: FilterQuality.low,
          )
        : RepaintBoundary(
            child: _DiscFace(cover: widget.cover, size: size),
          );

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
          ListenableBuilder(
            listenable: _turns,
            child: disc,
            builder: (context, child) {
              return Transform.rotate(
                angle: _turns.value * 6.283185307179586,
                child: child,
              );
            },
          ),
        ],
      ),
    );
  }
}

/// Offscreen disc rasterizer: one picture → one [ui.Image], no widget-tree races.
class _DiscBaker {
  const _DiscBaker._();

  static Future<ui.Image> bake({
    required int pixelSize,
    required double dpr,
    required ui.Image? cover,
  }) async {
    final recorder = ui.PictureRecorder();
    final canvas = Canvas(recorder);
    _paint(canvas, pixelSize: pixelSize.toDouble(), dpr: dpr, cover: cover);
    final picture = recorder.endRecording();
    try {
      return await picture.toImage(pixelSize, pixelSize);
    } finally {
      picture.dispose();
    }
  }

  static void _paint(
    Canvas canvas, {
    required double pixelSize,
    required double dpr,
    required ui.Image? cover,
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

    final coverRadius = radius * 0.72;
    final coverRect = Rect.fromCircle(center: center, radius: coverRadius);
    canvas.save();
    canvas.clipPath(Path()..addOval(coverRect));
    if (cover != null) {
      final src = Rect.fromLTWH(
        0,
        0,
        cover.width.toDouble(),
        cover.height.toDouble(),
      );
      canvas.drawImageRect(
        cover,
        src,
        coverRect,
        Paint()..filterQuality = FilterQuality.medium,
      );
    } else {
      // Match CoverView placeholder: diagonal gradient + music note
      // (centered gradient alone looks static while rotating).
      canvas.drawRect(
        coverRect,
        Paint()
          ..shader = StrawberryCream.coverPlaceholder.createShader(coverRect),
      );
      _paintMusicNote(canvas, center: center, iconSize: coverRadius * 2 * 0.38);
    }
    canvas.restore();

    final pinRadius = radius * 0.12;
    canvas.drawCircle(
      center,
      pinRadius,
      Paint()..color = StrawberryCream.cream,
    );
    canvas.drawCircle(
      center,
      pinRadius,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 2 * dpr
        ..color = StrawberryCream.strawberry.withValues(alpha: 0.5),
    );
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
