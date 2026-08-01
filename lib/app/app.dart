import 'dart:async';

import 'package:flutter/material.dart';
import 'package:mofu/app/router.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/library/library_controller.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:provider/provider.dart';

/// 草莓奶油播放器根应用：注入控制器、主题与路由。
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

class _MofuAppState extends State<MofuApp> {
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
    _ownsLibrary = widget.libraryController == null;
    _ownsPlayer = widget.playerController == null;
    _player.persistPlaybackMode = _library.persistPlaybackMode;
    _player.start();
    unawaited(_bootstrap());
  }

  Future<void> _bootstrap() async {
    await _library.load();
    try {
      final mode = await _library.loadPlaybackMode();
      if (!mounted) {
        return;
      }
      _player.setPlaybackMode(mode, persist: false);
    } catch (_) {
      // 曲库已加载；模式恢复失败时保持默认顺序播放。
    }
  }

  @override
  void dispose() {
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
        title: '草莓奶油',
        debugShowCheckedModeBanner: false,
        theme: StrawberryCream.theme(),
        routerConfig: _router,
      ),
    );
  }
}
