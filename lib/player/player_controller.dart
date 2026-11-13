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
  LyricsSessionSnapshot? get lyricsSession => _snapshot?.lyricsSession;
  EffectiveLyrics? get effectiveLyrics => _snapshot?.effectiveLyrics;

  /// Follow-up confirm state after Rust reports an existing-file or stem conflict.
  LyricsConfirmResult? get pendingLyricsConfirm => _pendingLyricsConfirm;
  LyricsConfirmResult? _pendingLyricsConfirm;
  LyricsConfirmResult? _stubbedConfirmResult;

  /// UI title: snapshot [displayTitle], else tag title, else idle placeholder.
  String get uiTitle {
    final fromSnapshot = displayTitle?.trim();
    if (fromSnapshot != null && fromSnapshot.isNotEmpty) {
      return fromSnapshot;
    }
    final fromTag = metadata?.title?.trim();
    if (fromTag != null && fromTag.isNotEmpty) {
      return fromTag;
    }
    return '未在播放';
  }

  /// UI artist with unknown fallback.
  String get uiArtist {
    final artist = metadata?.artist?.trim();
    if (artist != null && artist.isNotEmpty) {
      return artist;
    }
    return '未知歌手';
  }

  /// UI album with unknown fallback.
  String get uiAlbum {
    final album = metadata?.album?.trim();
    if (album != null && album.isNotEmpty) {
      return album;
    }
    return '未知专辑';
  }

  static PlayerSnapshot idleSnapshot({
    double volume = 1,
    PlaybackMode playbackMode = PlaybackMode.sequential,
  }) {
    return PlayerSnapshot(
      kind: PlayerSnapshotKind.full,
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
        _applySnapshot(next);
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

  void _applySnapshot(PlayerSnapshot next) {
    _snapshot = _retainHeavyFields(_snapshot, next);
    if (_snapshot?.lyricsSession == null) {
      _pendingLyricsConfirm = null;
    }
    _error = null;
    _retry = null;
    notifyListeners();
  }

  /// Keep cover/lyrics object identity when the track is unchanged.
  ///
  /// Progress ticks omit those fields. Full snapshots (play/pause) clone them
  /// across FFI; reusing the previous instances avoids tearing down the
  /// now-playing artwork widgets.
  static PlayerSnapshot _retainHeavyFields(
    PlayerSnapshot? previous,
    PlayerSnapshot next,
  ) {
    if (previous == null || previous.currentPath != next.currentPath) {
      return next;
    }

    if (next.kind == PlayerSnapshotKind.progress) {
      return _retainProgressFields(previous, next);
    }

    return _retainUnchangedHeavyFields(previous, next);
  }

  static PlayerSnapshot _retainProgressFields(
    PlayerSnapshot previous,
    PlayerSnapshot next,
  ) {
    final prevMeta = previous.metadata;
    final nextMeta = next.metadata;
    final TrackMetadata? mergedMeta;
    if (prevMeta != null && nextMeta != null) {
      mergedMeta = _copyMetadata(
        nextMeta,
        cover: prevMeta.cover,
        lyrics: prevMeta.lyrics,
      );
    } else {
      mergedMeta = prevMeta ?? nextMeta;
    }

    return _copy(
      next,
      kind: PlayerSnapshotKind.full,
      queue: previous.queue,
      upNext: previous.upNext,
      metadata: mergedMeta,
      lyricsSession: previous.lyricsSession,
      effectiveLyrics: previous.effectiveLyrics,
    );
  }

  static PlayerSnapshot _retainUnchangedHeavyFields(
    PlayerSnapshot previous,
    PlayerSnapshot next,
  ) {
    final prevMeta = previous.metadata;
    final nextMeta = next.metadata;
    if (prevMeta == null || nextMeta == null) {
      return next;
    }

    final cover = _sameCoverArt(prevMeta.cover, nextMeta.cover)
        ? prevMeta.cover
        : nextMeta.cover;
    final lyrics = _sameLyrics(prevMeta.lyrics, nextMeta.lyrics)
        ? prevMeta.lyrics
        : nextMeta.lyrics;
    final lyricsSession = previous.lyricsSession == next.lyricsSession
        ? previous.lyricsSession
        : next.lyricsSession;
    final effectiveLyrics = previous.effectiveLyrics == next.effectiveLyrics
        ? previous.effectiveLyrics
        : next.effectiveLyrics;
    if (identical(cover, nextMeta.cover) &&
        identical(lyrics, nextMeta.lyrics) &&
        identical(lyricsSession, next.lyricsSession) &&
        identical(effectiveLyrics, next.effectiveLyrics)) {
      return next;
    }

    return _copy(
      next,
      metadata: _copyMetadata(nextMeta, cover: cover, lyrics: lyrics),
      lyricsSession: lyricsSession,
      effectiveLyrics: effectiveLyrics,
    );
  }

  static TrackMetadata _copyMetadata(
    TrackMetadata source, {
    required CoverArt? cover,
    required List<Lyrics> lyrics,
  }) {
    return TrackMetadata(
      title: source.title,
      artist: source.artist,
      album: source.album,
      albumArtist: source.albumArtist,
      composer: source.composer,
      lyricist: source.lyricist,
      performer: source.performer,
      genre: source.genre,
      comment: source.comment,
      date: source.date,
      year: source.year,
      trackNumber: source.trackNumber,
      trackTotal: source.trackTotal,
      discNumber: source.discNumber,
      discTotal: source.discTotal,
      cover: cover,
      lyrics: lyrics,
    );
  }

  static bool _sameCoverArt(CoverArt? left, CoverArt? right) {
    if (identical(left, right)) {
      return true;
    }
    if (left == null || right == null) {
      return false;
    }
    if (left.mimeType != right.mimeType) {
      return false;
    }
    if (identical(left.data, right.data)) {
      return true;
    }
    return listEquals(left.data, right.data);
  }

  static bool _sameLyrics(List<Lyrics> left, List<Lyrics> right) {
    if (identical(left, right)) {
      return true;
    }
    return listEquals(left, right);
  }

  @visibleForTesting
  void emitSnapshot(PlayerSnapshot snapshot) {
    if (_disposed) {
      return;
    }
    _applySnapshot(snapshot);
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

  /// Forward a lyrics search for [path]. Rust owns matching and session state.
  Future<void> lyricsSearch(String path) async {
    if (_disposed) {
      return;
    }
    if (_fake) {
      recordedCommands.add('lyricsSearch:$path');
      notifyListeners();
      return;
    }
    final app = _app;
    if (app == null) {
      _error = '播放器仍在初始化';
      notifyListeners();
      return;
    }
    try {
      await app.lyricsSearch(path: path);
      if (_disposed) {
        return;
      }
      _error = null;
      _retry = null;
    } catch (err) {
      if (_disposed) {
        return;
      }
      _error = err.toString();
      _retry = () {
        unawaited(lyricsSearch(path));
      };
      notifyListeners();
    }
  }

  void lyricsPreview({required BigInt sessionId, required BigInt candidateId}) {
    _run(
      (app) =>
          app.lyricsPreview(sessionId: sessionId, candidateId: candidateId),
      command: 'lyricsPreview:$sessionId:$candidateId',
    );
  }

  void lyricsCancel({required BigInt sessionId}) {
    _pendingLyricsConfirm = null;
    _run(
      (app) => app.lyricsCancel(sessionId: sessionId),
      command: 'lyricsCancel:$sessionId',
    );
  }

  void lyricsPlayAndPreview({required BigInt sessionId, required String path}) {
    _run(
      (app) => app.lyricsPlayAndPreview(sessionId: sessionId, path: path),
      command: 'lyricsPlayAndPreview:$sessionId:$path',
    );
  }

  /// Test hook: next fake [lyricsConfirm] returns this result.
  @visibleForTesting
  void stubLyricsConfirmResult(LyricsConfirmResult result) {
    _stubbedConfirmResult = result;
  }

  Future<LyricsConfirmResult?> lyricsConfirm({
    required BigInt sessionId,
    required BigInt candidateId,
    required LyricsConfirmAction action,
  }) async {
    if (_disposed) {
      return null;
    }
    if (_fake) {
      recordedCommands.add(
        'lyricsConfirm:$sessionId:$candidateId:${action.name}',
      );
      final result =
          _stubbedConfirmResult ??
          LyricsConfirmResult(
            kind: LyricsConfirmResultKind.saved,
            audioPath: currentPath ?? '',
            conflictingAudio: const [],
          );
      _stubbedConfirmResult = null;
      if (result.kind == LyricsConfirmResultKind.saved) {
        _pendingLyricsConfirm = null;
      } else {
        _pendingLyricsConfirm = result;
      }
      notifyListeners();
      return result;
    }
    final app = _app;
    if (app == null) {
      _error = '播放器仍在初始化';
      notifyListeners();
      return null;
    }
    try {
      final result = await app.lyricsConfirm(
        sessionId: sessionId,
        candidateId: candidateId,
        action: action,
      );
      if (_disposed) {
        return result;
      }
      _error = null;
      _retry = null;
      if (result.kind == LyricsConfirmResultKind.saved) {
        _pendingLyricsConfirm = null;
      } else {
        _pendingLyricsConfirm = result;
      }
      notifyListeners();
      return result;
    } catch (err) {
      if (_disposed) {
        return null;
      }
      final message = err.toString();
      // Cancel / a replacement search may abandon an in-flight persist.
      // That is not a user-facing player error, even if a new session is live.
      if (message.contains('lyrics confirm was abandoned')) {
        return null;
      }
      _error = message;
      _retry = () {
        unawaited(
          lyricsConfirm(
            sessionId: sessionId,
            candidateId: candidateId,
            action: action,
          ),
        );
      };
      notifyListeners();
      return null;
    }
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
  PlayerSnapshotKind? kind,
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
  Object? lyricsSession = _unset,
  Object? effectiveLyrics = _unset,
}) {
  return PlayerSnapshot(
    kind: kind ?? current.kind,
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
    lyricsSession: identical(lyricsSession, _unset)
        ? current.lyricsSession
        : lyricsSession as LyricsSessionSnapshot?,
    effectiveLyrics: identical(effectiveLyrics, _unset)
        ? current.effectiveLyrics
        : effectiveLyrics as EffectiveLyrics?,
  );
}
