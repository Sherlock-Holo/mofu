import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:mofu/src/rust/api/library.dart' show PlaybackSession;
import 'package:mofu/src/rust/api/player.dart';

class _PendingQueue {
  const _PendingQueue({
    required this.paths,
    required this.startIndex,
    required this.autoplay,
    this.playlistId,
  });

  final List<String> paths;
  final BigInt? startIndex;
  final bool autoplay;
  final BigInt? playlistId;
}

/// 轮询 Rust [Player] 快照并转发播放控制。
class PlayerController extends ChangeNotifier {
  PlayerController({
    this._player,
    this._pollInterval = const Duration(milliseconds: 120),
  })
      : _ownsPlayer = true,
        _fake = false;

  /// 不连接 Rust / 音频设备，供 Widget 与 integration 测试注入。
  @visibleForTesting
  PlayerController.fake({
    PlayerSnapshot? snapshot,
    this._pollInterval = const Duration(days: 1),
  })  : _player = null,
        _ownsPlayer = false,
        _fake = true,
        _snapshot = snapshot ?? idleSnapshot(),
        _ready = Future<void>.value();

  Player? _player;
  final Duration _pollInterval;
  final bool _ownsPlayer;
  final bool _fake;

  Future<void>? _ready;
  _PendingQueue? _pendingQueue;

  /// Controls queued while [_player] is still null (seek / mode / volume / play…).
  List<void Function()>? _pendingOps;

  /// 用户切换播放模式时异步写库；启动恢复时不要挂副作用。
  Future<void> Function(PlaybackMode mode)? persistPlaybackMode;

  /// 音量 trailing debounce 写库；启动恢复时不要挂副作用。
  Future<void> Function(double volume)? persistVolume;

  /// 切歌 / 暂停 / 退出时写播放会话。
  Future<void> Function(PlaybackSession session)? persistPlaybackSession;

  /// 无有效队列或曲目时清除持久化会话。
  Future<void> Function()? clearPlaybackSession;

  /// 当前队列来源播放列表；用于会话恢复。
  BigInt? sourcePlaylistId;

  Timer? _pollTimer;
  Timer? _volumePersistTimer;
  double? _pendingVolume;
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

  /// 当前播放项路径；无队列或无选中项时为 null。
  String? get currentPath {
    final index = currentIndex;
    if (index == null) {
      return null;
    }
    for (final item in queue) {
      if (item.index == index) {
        return item.path;
      }
    }
    return null;
  }

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

  /// 确保底层 [Player] 已创建并开始轮询；并发调用共用同一 Future。
  Future<void> ensureReady() {
    if (_fake || _disposed) {
      return Future<void>.value();
    }
    final existing = _ready;
    if (existing != null) {
      return existing;
    }
    final future = Future<void>(() async {
      try {
        if (_disposed) {
          return;
        }
        _player ??= Player();
        start();
        _flushPending();
      } catch (err) {
        // 允许后续再次 ensureReady；勿把失败 Future 永久钉住。
        _ready = null;
        _error = err.toString();
        notifyListeners();
        rethrow;
      }
    });
    _ready = future;
    return future;
  }

  void start() {
    if (_fake) {
      return;
    }
    if (_player == null) {
      unawaited(ensureReady());
      return;
    }
    _pollTimer?.cancel();
    _refresh();
    _pollTimer = Timer.periodic(_pollInterval, (_) => _refresh());
  }

  /// 测试用：直接写入快照并通知监听者。
  @visibleForTesting
  void emitSnapshot(PlayerSnapshot snapshot) {
    final previous = _snapshot;
    _snapshot = snapshot;
    _error = null;
    _observeSessionTriggers(previous);
    notifyListeners();
  }

  void _refresh() {
    if (_disposed || _fake) {
      return;
    }
    final player = _player;
    if (player == null) {
      return;
    }
    try {
      final previous = _snapshot;
      final next = player.snapshot();
      final changed = previous != next;
      _snapshot = next;
      _observeSessionTriggers(previous);
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

  /// 切歌（含自动下一首）或 playing→paused 时写会话；播放中进度变化不写。
  void _observeSessionTriggers(PlayerSnapshot? previous) {
    final next = _snapshot;
    if (previous == null || next == null) {
      return;
    }
    final prevPath = _pathOf(previous);
    final nextPath = _pathOf(next);
    final trackChanged =
        previous.currentIndex != next.currentIndex || prevPath != nextPath;
    final pausedFromPlaying = previous.state == PlayerState.playing &&
        next.state == PlayerState.paused;
    if (trackChanged || pausedFromPlaying) {
      persistCurrentSession();
    }
  }

  String? _pathOf(PlayerSnapshot snapshot) {
    final index = snapshot.currentIndex;
    if (index == null) {
      return null;
    }
    for (final item in snapshot.queue) {
      if (item.index == index) {
        return item.path;
      }
    }
    return null;
  }

  void _run(void Function() action) {
    final previous = _snapshot;
    if (_fake) {
      try {
        action();
        _error = null;
        _observeSessionTriggers(previous);
        notifyListeners();
      } catch (err) {
        _error = err.toString();
        notifyListeners();
      }
      return;
    }
    if (_player == null) {
      (_pendingOps ??= []).add(action);
      unawaited(ensureReady());
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

  /// Ready 后先落 setQueue，再按序回放其它 pending 控制。
  void _flushPending() {
    _flushPendingQueue();
    final ops = _pendingOps;
    _pendingOps = null;
    if (ops == null || _player == null) {
      return;
    }
    for (final op in ops) {
      _run(op);
    }
  }

  void _flushPendingQueue() {
    final pending = _pendingQueue;
    _pendingQueue = null;
    if (pending == null || _player == null) {
      return;
    }
    if (pending.playlistId != null) {
      sourcePlaylistId = pending.playlistId;
    }
    _run(() {
      _player!.setQueue(
        paths: pending.paths,
        startIndex: pending.startIndex,
        autoplay: pending.autoplay,
      );
    });
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

  void setVolume(double volume, {bool persist = true}) => _run(() {
        final clamped = volume.clamp(0.0, 1.0);
        if (_fake) {
          final current = _snapshot ?? idleSnapshot();
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
        } else {
          _player!.setVolume(volume: volume);
        }
        if (persist) {
          _scheduleVolumePersist(clamped);
        }
      });

  void _scheduleVolumePersist(double volume) {
    _pendingVolume = volume;
    _volumePersistTimer?.cancel();
    _volumePersistTimer = Timer(const Duration(milliseconds: 400), () {
      final pending = _pendingVolume;
      _pendingVolume = null;
      _volumePersistTimer = null;
      final persist = persistVolume;
      if (pending != null && persist != null) {
        unawaited(persist(pending));
      }
    });
  }

  /// 退出 / 进后台时立刻落盘尚未 debounce 的音量。
  void flushVolumePersist() {
    _volumePersistTimer?.cancel();
    _volumePersistTimer = null;
    final pending = _pendingVolume;
    _pendingVolume = null;
    final persist = persistVolume;
    if (pending != null && persist != null) {
      unawaited(persist(pending));
    }
  }

  /// 将当前会话写入库；无有效 path / playlist 时清除。
  void persistCurrentSession() {
    final playlistId = sourcePlaylistId;
    final path = currentPath;
    if (playlistId == null || path == null || queue.isEmpty) {
      final clear = clearPlaybackSession;
      if (clear != null) {
        unawaited(clear());
      }
      return;
    }
    final persist = persistPlaybackSession;
    if (persist != null) {
      unawaited(
        persist(
          PlaybackSession(
            playlistId: playlistId,
            path: path,
            positionMs: positionMs,
          ),
        ),
      );
    }
  }

  void setPlaybackMode(PlaybackMode mode, {bool persist = true}) => _run(() {
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
        } else {
          _player!.setPlaybackMode(mode: mode);
        }
        if (persist) {
          final persistMode = persistPlaybackMode;
          if (persistMode != null) {
            unawaited(persistMode(mode));
          }
        }
      });

  void setQueue({
    required List<String> paths,
    BigInt? startIndex,
    bool autoplay = true,
    BigInt? playlistId,
  }) {
    if (playlistId != null) {
      sourcePlaylistId = playlistId;
    }
    if (!_fake && _player == null) {
      _pendingQueue = _PendingQueue(
        paths: List<String>.from(paths),
        startIndex: startIndex,
        autoplay: autoplay,
        playlistId: playlistId,
      );
      unawaited(ensureReady());
      return;
    }
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
  }

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
  void playPaths(
    List<String> paths, {
    required int startIndex,
    BigInt? playlistId,
  }) {
    if (paths.isEmpty) {
      return;
    }
    final clamped = startIndex.clamp(0, paths.length - 1);
    setQueue(
      paths: paths,
      startIndex: BigInt.from(clamped),
      autoplay: true,
      playlistId: playlistId,
    );
  }

  /// 播放列表全部曲目。随机模式下 Rust 会先打乱队列再从第一首顺序播放。
  void playAll(List<String> paths, {BigInt? playlistId}) {
    playPaths(paths, startIndex: 0, playlistId: playlistId);
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
    _volumePersistTimer?.cancel();
    _volumePersistTimer = null;
    _pendingQueue = null;
    _pendingOps = null;
    if (_ownsPlayer) {
      _player?.dispose();
    }
    super.dispose();
  }
}
