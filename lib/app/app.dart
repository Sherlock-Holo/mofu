import 'dart:async';

import 'package:flutter/material.dart';
import 'package:mofu/app/router.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/collection/collection_controller.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/src/rust/api/app.dart';
import 'package:mofu/src/rust/api/setup.dart';
import 'package:mofu/src/rust/frb_generated.dart';
import 'package:provider/provider.dart';

/// Mofu播放器根应用：注入控制器、主题与路由。
class MofuApp extends StatefulWidget {
  const MofuApp({super.key, this.collectionController, this.playerController})
    : assert(
        (collectionController == null) == (playerController == null),
        'Inject both controllers together, or neither.',
      );

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
  var _bootstrapping = false;
  Object? _bootstrapError;
  App? _app;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    _ownsCollection = widget.collectionController == null;
    _ownsPlayer = widget.playerController == null;
    _bootstrapping = _ownsCollection || _ownsPlayer;
    unawaited(_bootstrap());
  }

  Future<void> _bootstrap() async {
    // 测试注入 fake 时跳过 Rust 初始化。
    if (!_ownsCollection && !_ownsPlayer) {
      return;
    }

    if (mounted && !_bootstrapping) {
      setState(() {
        _bootstrapping = true;
        _bootstrapError = null;
      });
    }
    try {
      await RustLib.init();
      setup();
      final (app, snapshot) = await App.bootstrap();
      if (!mounted) {
        app.shutdown();
        app.dispose();
        return;
      }
      _app = app;
      _collection.attach(
        app,
        initial: snapshot.library_,
        selectedPlaylistId: snapshot.selectedPlaylistId,
      );
      _player.attach(app, initial: snapshot.player);
      if (mounted) {
        setState(() {
          _bootstrapping = false;
          _bootstrapError = null;
        });
      }
    } catch (err, stack) {
      debugPrint('bootstrap failed: $err\n$stack');
      final app = _app;
      _app = null;
      if (app != null) {
        app.shutdown();
        app.dispose();
      }
      if (mounted) {
        setState(() {
          _bootstrapping = false;
          _bootstrapError = err;
        });
      }
    }
  }

  void _flushPersist() {
    _player.flush();
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
    final app = _app;
    _app = null;
    if (app != null) {
      app.shutdown();
      app.dispose();
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
        builder: (context, child) {
          if (_bootstrapping) {
            return const Scaffold(
              body: Center(child: CircularProgressIndicator()),
            );
          }
          final error = _bootstrapError;
          if (error != null) {
            return Scaffold(
              body: Center(
                child: ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 420),
                  child: Padding(
                    padding: const EdgeInsets.all(24),
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        const Icon(Icons.error_outline_rounded, size: 48),
                        const SizedBox(height: 16),
                        const Text('Mofu 启动失败'),
                        const SizedBox(height: 8),
                        Text(
                          error.toString(),
                          textAlign: TextAlign.center,
                          maxLines: 4,
                          overflow: TextOverflow.ellipsis,
                        ),
                        const SizedBox(height: 20),
                        FilledButton.icon(
                          onPressed: _bootstrap,
                          icon: const Icon(Icons.refresh_rounded),
                          label: const Text('重试'),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            );
          }
          return child ?? const SizedBox.shrink();
        },
      ),
    );
  }
}
