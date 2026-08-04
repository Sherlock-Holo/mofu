import 'package:flutter/material.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/features/home/widgets/current_track_header.dart';
import 'package:mofu/features/home/widgets/playlist_sidebar.dart';
import 'package:mofu/features/home/widgets/song_table.dart';
import 'package:mofu/collection/collection_controller.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/widgets/playback_bar.dart';
import 'package:provider/provider.dart';

/// 首页：侧栏播放列表 + 浏览列表摘要 + 曲目表 + 底栏。
class HomePage extends StatelessWidget {
  const HomePage({super.key});

  static const _sidebarBreakpoint = 880.0;

  @override
  Widget build(BuildContext context) {
    final collection = context.watch<CollectionController>();
    // 只订阅错误文案；进度 tick 不应重建整页（含子树 SongTable）。
    final playerError = context.select<PlayerController, String?>(
      (player) => player.error,
    );
    final wide = MediaQuery.sizeOf(context).width >= _sidebarBreakpoint;

    final body = Row(
      children: [
        if (wide)
          const SizedBox(
            width: 220,
            child: PlaylistSidebar(),
          ),
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.stretch,
            children: [
              if (collection.error != null || playerError != null)
                _ErrorBanner(
                  message: collection.error ?? playerError!,
                  onDismiss: () {
                    // 错误在下次成功操作时清除；这里触发一次刷新。
                    collection.load();
                  },
                ),
              const CurrentTrackHeader(),
              const Expanded(child: SongTable()),
            ],
          ),
        ),
      ],
    );

    return Scaffold(
      appBar: wide
          ? null
          : AppBar(
              title: const Text('Mofu'),
              actions: [
                IconButton(
                  tooltip: '打开音乐目录',
                  onPressed:
                      collection.loading ? null : collection.pickAndAddDirectory,
                  icon: const Icon(Icons.folder_open_rounded),
                ),
                IconButton(
                  tooltip: '添加音乐文件',
                  onPressed: collection.loading ? null : collection.pickAndAddFiles,
                  icon: const Icon(Icons.library_add_rounded),
                ),
              ],
            ),
      drawer: wide
          ? null
          : const Drawer(
              backgroundColor: StrawberryCream.cream,
              child: SafeArea(child: PlaylistSidebar()),
            ),
      body: body,
      bottomNavigationBar: const PlaybackBar(),
    );
  }
}

class _ErrorBanner extends StatelessWidget {
  const _ErrorBanner({
    required this.message,
    required this.onDismiss,
  });

  final String message;
  final VoidCallback onDismiss;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: StrawberryCream.strawberry.withValues(alpha: 0.14),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 16, vertical: 10),
        child: Row(
          children: [
            const Icon(Icons.error_outline_rounded,
                color: StrawberryCream.strawberry),
            const SizedBox(width: 10),
            Expanded(
              child: Text(
                message,
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
                style: const TextStyle(color: StrawberryCream.wine),
              ),
            ),
            TextButton(onPressed: onDismiss, child: const Text('重试')),
          ],
        ),
      ),
    );
  }
}
