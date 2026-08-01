import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:mofu/src/rust/api/player.dart';

/// 轮询 Rust [Player] 快照并转发播放控制。
class PlayerController extends ChangeNotifier {
  PlayerController({
    Player? player,
    Duration pollInterval = const Duration(milliseconds: 120),
  })  : _player = player ?? Player(),
        _pollInterval = pollInterval,
        _ownsPlayer = true,
        _fake = false;

  /// 不连接 Rust / 音频设备，供 Widget 与 integration 测试注入。
  @visibleForTesting
  PlayerController.fake({
    PlayerSnapshot? snapshot,
    Duration pollInterval = const Duration(days: 1),
  })  : _player = null,
        _pollInterval = pollInterval,
        _ownsPlayer = false,
        _fake = true,
        _snapshot = snapshot ?? idleSnapshot();

  final Player? _player;
  final Duration _pollInterval;
  final bool _ownsPlayer;
  final bool _fake;

  Timer? _pollTimer;
  PlayerSnapshot? _snapshot;
  String? _error;
  bool _disposed = false;

  PlayerSnapshot? get snapshot => _snapshot;
  String? get error => _error;
  bool get isPlaying => _snapshot?.state == PlayerState.playing;
  TrackMetadata? get metadata => _snapshot?.metadata;
  BigInt get positionMs => _snapshot?.positionMs ?? BigInt.zero;
  BigInt get durationMs => _snapshot?.durationMs ?? BigInt.zero;
  double get volume => _snapshot?.volume ?? 1;
  PlaybackMode get playbackMode =>
      _snapshot?.playbackMode ?? PlaybackMode.sequential;
  BigInt? get currentIndex => _snapshot?.currentIndex;
  List<QueueItem> get queue => _snapshot?.queue ?? const [];

  /// 空闲快照，供假控制器与测试夹具复用。
  static PlayerSnapshot idleSnapshot({
    double volume = 1,
    PlaybackMode playbackMode = PlaybackMode.sequential,
  }) {
    return PlayerSnapshot(
      positionMs: BigInt.zero,
      durationMs: BigInt.zero,
      state: PlayerState.idle,
      volume: volume,
      playbackMode: playbackMode,
      queue: const [],
    );
  }

  void start() {
    if (_fake) {
      return;
    }
    _pollTimer?.cancel();
    _refresh();
    _pollTimer = Timer.periodic(_pollInterval, (_) => _refresh());
  }

  /// 测试用：直接写入快照并通知监听者。
  @visibleForTesting
  void emitSnapshot(PlayerSnapshot snapshot) {
    _snapshot = snapshot;
    _error = null;
    notifyListeners();
  }

  void _refresh() {
    if (_disposed || _fake) {
      return;
    }
    try {
      final next = _player!.snapshot();
      final changed = _snapshot != next;
      _snapshot = next;
      if (_error != null) {
        _error = null;
        notifyListeners();
        return;
      }
      if (changed) {
        notifyListeners();
      }
    } catch (err) {
      _error = err.toString();
      notifyListeners();
    }
  }

  void _run(void Function() action) {
    if (_fake) {
      try {
        action();
        _error = null;
        notifyListeners();
      } catch (err) {
        _error = err.toString();
        notifyListeners();
      }
      return;
    }
    try {
      action();
      _error = null;
      _refresh();
    } catch (err) {
      _error = err.toString();
      notifyListeners();
    }
  }

  void play() => _run(() {
        if (_fake) {
          final current = _snapshot ?? idleSnapshot();
          _snapshot = PlayerSnapshot(
            positionMs: current.positionMs,
            durationMs: current.durationMs,
            state: PlayerState.playing,
            volume: current.volume,
            playbackMode: current.playbackMode,
            currentIndex: current.currentIndex,
            queue: current.queue,
            metadata: current.metadata,
          );
          return;
        }
        _player!.play();
      });

  void pause() => _run(() {
        if (_fake) {
          final current = _snapshot ?? idleSnapshot();
          _snapshot = PlayerSnapshot(
            positionMs: current.positionMs,
            durationMs: current.durationMs,
            state: PlayerState.paused,
            volume: current.volume,
            playbackMode: current.playbackMode,
            currentIndex: current.currentIndex,
            queue: current.queue,
            metadata: current.metadata,
          );
          return;
        }
        _player!.pause();
      });

  void stop() => _run(() {
        if (_fake) {
          final current = _snapshot ?? idleSnapshot();
          _snapshot = PlayerSnapshot(
            positionMs: BigInt.zero,
            durationMs: current.durationMs,
            state: PlayerState.idle,
            volume: current.volume,
            playbackMode: current.playbackMode,
            currentIndex: null,
            queue: current.queue,
            metadata: null,
          );
          return;
        }
        _player!.stop();
      });

  void togglePlayPause() {
    if (isPlaying) {
      pause();
    } else {
      play();
    }
  }

  void next() => _run(() {
        if (_fake) {
          _stepQueue(1);
          return;
        }
        _player!.next();
      });

  void previous() => _run(() {
        if (_fake) {
          _stepQueue(-1);
          return;
        }
        _player!.previous();
      });

  void seek(BigInt positionMs) => _run(() {
        if (_fake) {
          final current = _snapshot ?? idleSnapshot();
          final duration = current.durationMs;
          var next = positionMs;
          if (next < BigInt.zero) {
            next = BigInt.zero;
          }
          if (duration > BigInt.zero && next > duration) {
            next = duration;
          }
          _snapshot = PlayerSnapshot(
            positionMs: next,
            durationMs: current.durationMs,
            state: current.state,
            volume: current.volume,
            playbackMode: current.playbackMode,
            currentIndex: current.currentIndex,
            queue: current.queue,
            metadata: current.metadata,
          );
          return;
        }
        _player!.seek(positionMs: positionMs);
      });

  void seekRelative(int deltaMs) {
    if (_fake) {
      seek(positionMs + BigInt.from(deltaMs));
      return;
    }
    _run(
      () => _player!.seekRelative(deltaMs: PlatformInt64Util.from(deltaMs)),
    );
  }

  void setVolume(double volume) => _run(() {
        if (_fake) {
          final current = _snapshot ?? idleSnapshot();
          final clamped = volume.clamp(0.0, 1.0);
          _snapshot = PlayerSnapshot(
            positionMs: current.positionMs,
            durationMs: current.durationMs,
            state: current.state,
            volume: clamped,
            playbackMode: current.playbackMode,
            currentIndex: current.currentIndex,
            queue: current.queue,
            metadata: current.metadata,
          );
          return;
        }
        _player!.setVolume(volume: volume);
      });

  void setPlaybackMode(PlaybackMode mode) => _run(() {
        if (_fake) {
          final current = _snapshot ?? idleSnapshot();
          _snapshot = PlayerSnapshot(
            positionMs: current.positionMs,
            durationMs: current.durationMs,
            state: current.state,
            volume: current.volume,
            playbackMode: mode,
            currentIndex: current.currentIndex,
            queue: current.queue,
            metadata: current.metadata,
          );
          return;
        }
        _player!.setPlaybackMode(mode: mode);
      });

  void setQueue({
    required List<String> paths,
    BigInt? startIndex,
    bool autoplay = true,
  }) =>
      _run(() {
        if (_fake) {
          final queue = <QueueItem>[
            for (var i = 0; i < paths.length; i++)
              QueueItem(index: BigInt.from(i), path: paths[i]),
          ];
          BigInt? index;
          if (queue.isNotEmpty) {
            final raw = startIndex?.toInt() ?? 0;
            final clamped = raw.clamp(0, queue.length - 1);
            index = BigInt.from(clamped);
          }
          final current = _snapshot ?? idleSnapshot();
          _snapshot = PlayerSnapshot(
            positionMs: BigInt.zero,
            durationMs: current.durationMs,
            state: autoplay && index != null
                ? PlayerState.playing
                : PlayerState.ready,
            volume: current.volume,
            playbackMode: current.playbackMode,
            currentIndex: index,
            queue: queue,
            metadata: current.metadata,
          );
          return;
        }
        _player!.setQueue(
          paths: paths,
          startIndex: startIndex,
          autoplay: autoplay,
        );
      });

  void playIndex(BigInt index) => _run(() {
        if (_fake) {
          final current = _snapshot ?? idleSnapshot();
          if (current.queue.isEmpty) {
            return;
          }
          final clamped = index.toInt().clamp(0, current.queue.length - 1);
          _snapshot = PlayerSnapshot(
            positionMs: BigInt.zero,
            durationMs: current.durationMs,
            state: PlayerState.playing,
            volume: current.volume,
            playbackMode: current.playbackMode,
            currentIndex: BigInt.from(clamped),
            queue: current.queue,
            metadata: current.metadata,
          );
          return;
        }
        _player!.playIndex(index: index);
      });

  /// 用整份列表路径替换队列并播放指定项。
  void playPaths(List<String> paths, {required int startIndex}) {
    if (paths.isEmpty) {
      return;
    }
    final clamped = startIndex.clamp(0, paths.length - 1);
    setQueue(
      paths: paths,
      startIndex: BigInt.from(clamped),
      autoplay: true,
    );
  }

  /// 播放列表全部曲目；起播位置按当前播放模式选择（随机模式随机一首，其余从第一首）。
  void playAll(List<String> paths) {
    if (paths.isEmpty) {
      return;
    }
    final startIndex = switch (playbackMode) {
      PlaybackMode.shuffle => _randomIndex(paths.length),
      PlaybackMode.sequential ||
      PlaybackMode.repeatOne ||
      PlaybackMode.repeatAll =>
        0,
    };
    playPaths(paths, startIndex: startIndex);
  }

  int _randomIndex(int length) {
    if (length <= 1) {
      return 0;
    }
    return DateTime.now().microsecondsSinceEpoch.abs() % length;
  }

  void _stepQueue(int delta) {
    final current = _snapshot ?? idleSnapshot();
    if (current.queue.isEmpty || current.currentIndex == null) {
      return;
    }
    final next = current.currentIndex!.toInt() + delta;
    if (next < 0 || next >= current.queue.length) {
      return;
    }
    _snapshot = PlayerSnapshot(
      positionMs: BigInt.zero,
      durationMs: current.durationMs,
      state: PlayerState.playing,
      volume: current.volume,
      playbackMode: current.playbackMode,
      currentIndex: BigInt.from(next),
      queue: current.queue,
      metadata: current.metadata,
    );
  }

  @override
  void dispose() {
    _disposed = true;
    _pollTimer?.cancel();
    if (_ownsPlayer) {
      _player?.dispose();
    }
    super.dispose();
  }
}
