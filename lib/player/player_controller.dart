import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:mofu/src/rust/api/app.dart';
import 'package:mofu/src/rust/api/player.dart';

/// Thin UI adapter: subscribes to Rust player snapshots and forwards controls.
class PlayerController extends ChangeNotifier {
  PlayerController() : _fake = false;

  /// Test double that records commands and accepts injected snapshots.
  @visibleForTesting
  PlayerController.fake({PlayerSnapshot? snapshot})
    : _fake = true,
      _snapshot = snapshot ?? idleSnapshot();

  App? _app;
  StreamSubscription<PlayerSnapshot>? _subscription;
  final bool _fake;
  PlayerSnapshot? _snapshot;
  String? _error;
  VoidCallback? _retry;
  bool _disposed = false;

  /// Commands recorded by the fake controller for widget tests.
  @visibleForTesting
  final List<String> recordedCommands = [];

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
  String? get currentPath => _snapshot?.currentPath;
  String? get displayTitle => _snapshot?.displayTitle;
  BigInt? get activeLyricsLine => _snapshot?.activeLyricsLine;

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
      currentFromUpNext: false,
      queue: const [],
      upNext: const [],
    );
  }

  /// Bind to the shared Rust [App] and start the snapshot stream.
  void attach(App app, {PlayerSnapshot? initial}) {
    if (_fake || _disposed) {
      return;
    }
    _app = app;
    if (initial != null) {
      _snapshot = initial;
    } else {
      _snapshot = app.playerSnapshot();
    }
    _subscribe();
    notifyListeners();
  }

  void _subscribe() {
    final app = _app;
    if (app == null || _disposed) {
      return;
    }
    unawaited(_subscription?.cancel());
    _subscription = app.watchPlayer().listen(
      (next) {
        if (_disposed) {
          return;
        }
        _snapshot = next;
        _error = null;
        _retry = null;
        notifyListeners();
      },
      onError: (Object err) {
        if (_disposed) {
          return;
        }
        _error = err.toString();
        _retry = _subscribe;
        notifyListeners();
      },
    );
  }

  @visibleForTesting
  void emitSnapshot(PlayerSnapshot snapshot) {
    if (_disposed) {
      return;
    }
    _snapshot = snapshot;
    _error = null;
    notifyListeners();
  }

  void _run(void Function(App app) action, {required String command}) {
    if (_disposed) {
      return;
    }
    if (_fake) {
      recordedCommands.add(command);
      _applyFake(command);
      return;
    }
    final app = _app;
    if (app == null) {
      _error = '播放器仍在初始化';
      _retry = null;
      notifyListeners();
      return;
    }
    try {
      final hadError = _error != null;
      action(app);
      _error = null;
      _retry = null;
      if (hadError) {
        notifyListeners();
      }
    } catch (err) {
      _error = err.toString();
      _retry = () => _run(action, command: command);
      notifyListeners();
    }
  }

  void _applyFake(String command) {
    final current = _snapshot ?? idleSnapshot();
    switch (command) {
      case 'play':
        _snapshot = _copy(current, state: PlayerState.playing);
      case 'pause':
        _snapshot = _copy(current, state: PlayerState.paused);
      case 'stop':
        _snapshot = _copy(
          current,
          positionMs: BigInt.zero,
          state: PlayerState.idle,
          currentIndex: null,
          currentFromUpNext: false,
          currentPath: null,
          displayTitle: null,
          metadata: null,
        );
      default:
        break;
    }
    notifyListeners();
  }

  void play() => _run((app) => app.play(), command: 'play');

  void pause() => _run((app) => app.pause(), command: 'pause');

  void stop() => _run((app) => app.stop(), command: 'stop');

  void togglePlayPause() {
    if (isPlaying) {
      pause();
    } else {
      play();
    }
  }

  void next() => _run((app) => app.next(), command: 'next');

  void previous() => _run((app) => app.previous(), command: 'previous');

  void seek(BigInt positionMs) =>
      _run((app) => app.seek(positionMs: positionMs), command: 'seek');

  void seekRelative(int deltaMs) {
    if (_fake) {
      recordedCommands.add('seekRelative');
      final current = _snapshot ?? idleSnapshot();
      var next = positionMs + BigInt.from(deltaMs);
      if (next < BigInt.zero) {
        next = BigInt.zero;
      }
      final duration = current.durationMs;
      if (duration > BigInt.zero && next > duration) {
        next = duration;
      }
      _snapshot = _copy(current, positionMs: next);
      notifyListeners();
      return;
    }
    _run(
      (app) => app.seekRelative(deltaMs: PlatformInt64Util.from(deltaMs)),
      command: 'seekRelative',
    );
  }

  void setVolume(double volume) {
    final clamped = volume.clamp(0.0, 1.0);
    if (_fake) {
      recordedCommands.add('setVolume');
      final current = _snapshot ?? idleSnapshot();
      _snapshot = _copy(current, volume: clamped);
      notifyListeners();
      return;
    }
    _run((app) => app.setVolume(volume: clamped), command: 'setVolume');
  }

  void setPlaybackMode(PlaybackMode mode) {
    if (_fake) {
      recordedCommands.add('setPlaybackMode');
      final current = _snapshot ?? idleSnapshot();
      _snapshot = _copy(current, playbackMode: mode);
      notifyListeners();
      return;
    }
    _run((app) => app.setPlaybackMode(mode: mode), command: 'setPlaybackMode');
  }

  void enqueueNext(String path) {
    if (_fake) {
      recordedCommands.add('enqueueNext:$path');
      final current = _snapshot ?? idleSnapshot();
      _snapshot = _copy(
        current,
        upNext: [
          ...current.upNext,
          QueueItem(index: BigInt.from(current.upNext.length), path: path),
        ],
      );
      notifyListeners();
      return;
    }
    _run((app) => app.enqueueNext(path: path), command: 'enqueueNext');
  }

  Future<void> playPlaylist({
    required BigInt playlistId,
    String? startPath,
    bool autoplay = true,
  }) async {
    if (_fake) {
      recordedCommands.add('playPlaylist:$playlistId:$startPath:$autoplay');
      notifyListeners();
      return;
    }
    final app = _app;
    if (app == null) {
      if (!_disposed) {
        _error = '播放器仍在初始化';
        notifyListeners();
      }
      return;
    }
    try {
      await app.playPlaylist(
        playlistId: playlistId,
        startPath: startPath,
        autoplay: autoplay,
      );
      if (_disposed) {
        return;
      }
      final hadError = _error != null;
      _error = null;
      _retry = null;
      if (hadError) {
        notifyListeners();
      }
    } catch (err) {
      if (_disposed) {
        return;
      }
      _error = err.toString();
      _retry = () {
        unawaited(
          playPlaylist(
            playlistId: playlistId,
            startPath: startPath,
            autoplay: autoplay,
          ),
        );
      };
      notifyListeners();
    }
  }

  Future<void> playAll({required BigInt playlistId}) {
    return playPlaylist(playlistId: playlistId);
  }

  Future<void> playIndex({required BigInt playlistId, required String path}) {
    return playPlaylist(playlistId: playlistId, startPath: path);
  }

  void retryLastError() {
    if (_disposed) {
      return;
    }
    _retry?.call();
  }

  void flush() {
    if (_fake || _disposed) {
      return;
    }
    _app?.flush();
  }

  @override
  void dispose() {
    _disposed = true;
    _subscription?.cancel();
    _subscription = null;
    _retry = null;
    super.dispose();
  }
}

const _unset = Object();

PlayerSnapshot _copy(
  PlayerSnapshot current, {
  BigInt? positionMs,
  BigInt? durationMs,
  PlayerState? state,
  double? volume,
  PlaybackMode? playbackMode,
  Object? currentIndex = _unset,
  bool? currentFromUpNext,
  Object? currentPath = _unset,
  Object? displayTitle = _unset,
  Object? activeLyricsLine = _unset,
  Object? sourcePlaylistId = _unset,
  List<QueueItem>? queue,
  List<QueueItem>? upNext,
  Object? metadata = _unset,
}) {
  return PlayerSnapshot(
    positionMs: positionMs ?? current.positionMs,
    durationMs: durationMs ?? current.durationMs,
    state: state ?? current.state,
    volume: volume ?? current.volume,
    playbackMode: playbackMode ?? current.playbackMode,
    currentIndex: identical(currentIndex, _unset)
        ? current.currentIndex
        : currentIndex as BigInt?,
    currentFromUpNext: currentFromUpNext ?? current.currentFromUpNext,
    currentPath: identical(currentPath, _unset)
        ? current.currentPath
        : currentPath as String?,
    displayTitle: identical(displayTitle, _unset)
        ? current.displayTitle
        : displayTitle as String?,
    activeLyricsLine: identical(activeLyricsLine, _unset)
        ? current.activeLyricsLine
        : activeLyricsLine as BigInt?,
    sourcePlaylistId: identical(sourcePlaylistId, _unset)
        ? current.sourcePlaylistId
        : sourcePlaylistId as BigInt?,
    queue: queue ?? current.queue,
    upNext: upNext ?? current.upNext,
    metadata: identical(metadata, _unset)
        ? current.metadata
        : metadata as TrackMetadata?,
  );
}
