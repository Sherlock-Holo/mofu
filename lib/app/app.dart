import 'dart:async';

import 'package:flutter/material.dart';
import 'package:mofu/app/router.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/collection/collection_controller.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/src/rust/api/collection.dart';
import 'package:mofu/src/rust/api/setup.dart';
import 'package:mofu/src/rust/frb_generated.dart';
import 'package:provider/provider.dart';

/// Mofu播放器根应用：注入控制器、主题与路由。
class MofuApp extends StatefulWidget {
  const MofuApp({
    super.key,
    this.collectionController,
    this.playerController,
  });

  final CollectionController? collectionController;
  final PlayerController? playerController;

  @override
  State<MofuApp> createState() => _MofuAppState();
}

class _MofuAppState extends State<MofuApp> with WidgetsBindingObserver {
  late final CollectionController _collection =
      widget.collectionController ?? CollectionController();
  late final PlayerController _player =
      widget.playerController ?? PlayerController();
  late final _router = createRouter();
  var _ownsCollection = false;
  var _ownsPlayer = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _ownsCollection = widget.collectionController == null;
    _ownsPlayer = widget.playerController == null;
    _player.persistPlaybackMode = _collection.persistPlaybackMode;
    _player.persistVolume = _collection.persistVolume;
    _player.persistPlaybackSession = _collection.persistPlaybackSession;
    _player.clearPlaybackSession = _collection.clearPlaybackSession;
    unawaited(_bootstrap());
  }

  Future<void> _bootstrap() async {
    // 生产路径自建 controller，需要先加载 FRB；测试注入 fake 时跳过。
    if (_ownsCollection || _ownsPlayer) {
      await RustLib.init();
      setup();
    }
    if (!mounted) {
      return;
    }

    final playerReady = _player.ensureReady();
    await _collection.load();
    if (!mounted) {
      return;
    }

    await playerReady;
    if (!mounted) {
      return;
    }

    // 先顺序读完 meta（音量/模式/会话），再异步补封面。
    // 连接池已支持并发库操作；此处顺序加载仍作防御性编排，避免启动路径竞态。
    try {
      final volume = await _collection.loadVolume();
      if (!mounted) {
        return;
      }
      _player.setVolume(volume, persist: false);
    } catch (err, stack) {
      // 音量恢复失败时保持默认。
      debugPrint('bootstrap: loadVolume failed: $err\n$stack');
    }

    try {
      final mode = await _collection.loadPlaybackMode();
      if (!mounted) {
        return;
      }
      _player.setPlaybackMode(mode, persist: false);
    } catch (err, stack) {
      // 曲库已加载；模式恢复失败时保持默认顺序播放。
      debugPrint('bootstrap: loadPlaybackMode failed: $err\n$stack');
    }

    try {
      final session = await _collection.loadPlaybackSession();
      if (!mounted || session == null) {
        unawaited(_collection.hydrateCovers());
        return;
      }
      await _restoreSession(session);
    } catch (err, stack) {
      // 会话恢复失败时保持空闲，不打断启动。
      debugPrint('bootstrap: loadPlaybackSession failed: $err\n$stack');
    }

    unawaited(_collection.hydrateCovers());
  }

  Future<void> _restoreSession(PlaybackSession session) async {
    Playlist? playlist;
    for (final candidate in _collection.playlists) {
      if (candidate.id == session.playlistId) {
        playlist = candidate;
        break;
      }
    }
    if (playlist == null) {
      await _collection.clearPlaybackSession();
      return;
    }

    final paths = playlist.songs.map((song) => song.path).toList();
    final index = paths.indexOf(session.path);
    if (index < 0) {
      await _collection.clearPlaybackSession();
      return;
    }

    // 恢复过程不要把 position 写成 0 覆盖库里的会话。
    final persistSession = _player.persistPlaybackSession;
    final clearSession = _player.clearPlaybackSession;
    _player.persistPlaybackSession = null;
    _player.clearPlaybackSession = null;
    try {
      _player.setQueue(
        paths: paths,
        startIndex: BigInt.from(index),
        autoplay: false,
        playlistId: playlist.id,
      );
      _player.seek(session.positionMs);
      _collection.selectPlaylist(playlist.id);
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
    if (_ownsCollection) {
      _collection.dispose();
    }
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return MultiProvider(
      providers: [
        ChangeNotifierProvider<CollectionController>.value(value: _collection),
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
