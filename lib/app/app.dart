import 'dart:async';

import 'package:flutter/material.dart';
import 'package:mofu/app/router.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/library/library_controller.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/src/rust/api/library.dart';
import 'package:mofu/src/rust/api/player.dart' show PlaybackMode;
import 'package:provider/provider.dart';

/// Mofu播放器根应用：注入控制器、主题与路由。
class MofuApp extends StatefulWidget {
  const MofuApp({
    super.key,
    this.libraryController,
    this.playerController,
  });

  final LibraryController? libraryController;
  final PlayerController? playerController;

  @override
  State<MofuApp> createState() => _MofuAppState();
}

class _MofuAppState extends State<MofuApp> with WidgetsBindingObserver {
  late final LibraryController _library =
      widget.libraryController ?? LibraryController();
  late final PlayerController _player =
      widget.playerController ?? PlayerController();
  late final _router = createRouter();
  var _ownsLibrary = false;
  var _ownsPlayer = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _ownsLibrary = widget.libraryController == null;
    _ownsPlayer = widget.playerController == null;
    _player.persistPlaybackMode = _library.persistPlaybackMode;
    _player.persistVolume = _library.persistVolume;
    _player.persistPlaybackSession = _library.persistPlaybackSession;
    _player.clearPlaybackSession = _library.clearPlaybackSession;
    _player.start();
    unawaited(_bootstrap());
  }

  Future<void> _bootstrap() async {
    await _library.load();
    if (!mounted) {
      return;
    }

    try {
      final volume = await _library.loadVolume();
      if (!mounted) {
        return;
      }
      _player.setVolume(volume, persist: false);
    } catch (_) {
      // 音量恢复失败时保持默认。
    }

    try {
      final mode = await _library.loadPlaybackMode();
      if (!mounted) {
        return;
      }
      _player.setPlaybackMode(mode, persist: false);
    } catch (_) {
      // 曲库已加载；模式恢复失败时保持默认顺序播放。
    }

    try {
      final session = await _library.loadPlaybackSession();
      if (!mounted || session == null) {
        return;
      }
      await _restoreSession(session);
    } catch (_) {
      // 会话恢复失败时保持空闲，不打断启动。
    }
  }

  Future<void> _restoreSession(PlaybackSession session) async {
    Playlist? playlist;
    for (final candidate in _library.playlists) {
      if (candidate.id == session.playlistId) {
        playlist = candidate;
        break;
      }
    }
    if (playlist == null) {
      await _library.clearPlaybackSession();
      return;
    }

    final paths = playlist.songs.map((song) => song.path).toList();
    final index = paths.indexOf(session.path);
    if (index < 0) {
      await _library.clearPlaybackSession();
      return;
    }

    // 恢复过程不要把 position 写成 0 覆盖库里的会话。
    final persistSession = _player.persistPlaybackSession;
    final clearSession = _player.clearPlaybackSession;
    _player.persistPlaybackSession = null;
    _player.clearPlaybackSession = null;
    try {
      // 会话存的是 path；但 Shuffle 下 queue.replace 会打乱并强制从下标 0 起播，
      // 忽略 startIndex。先按顺序装入目标曲，再切回 shuffle（set_mode 会保留当前 path）。
      final wasShuffle = _player.playbackMode == PlaybackMode.shuffle;
      if (wasShuffle) {
        _player.setPlaybackMode(PlaybackMode.sequential, persist: false);
      }
      _player.setQueue(
        paths: paths,
        startIndex: BigInt.from(index),
        autoplay: false,
        playlistId: playlist.id,
      );
      _player.seek(session.positionMs);
      if (wasShuffle) {
        _player.setPlaybackMode(PlaybackMode.shuffle, persist: false);
      }
      _library.selectPlaylist(playlist.id);
    } finally {
      _player.persistPlaybackSession = persistSession;
      _player.clearPlaybackSession = clearSession;
    }
  }

  void _flushPersist() {
    _player.flushVolumePersist();
    _player.persistCurrentSession();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.paused ||
        state == AppLifecycleState.detached) {
      _flushPersist();
    }
  }

  @override
  void dispose() {
    WidgetsBinding.instance.removeObserver(this);
    _flushPersist();
    if (_ownsPlayer) {
      _player.dispose();
    }
    if (_ownsLibrary) {
      _library.dispose();
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return MultiProvider(
      providers: [
        ChangeNotifierProvider<LibraryController>.value(value: _library),
        ChangeNotifierProvider<PlayerController>.value(value: _player),
      ],
      child: MaterialApp.router(
        title: 'Mofu',
        debugShowCheckedModeBanner: false,
        theme: StrawberryCream.theme(),
        routerConfig: _router,
      ),
    );
  }
}
