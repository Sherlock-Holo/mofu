import 'package:flutter/material.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/collection/collection_controller.dart';
import 'package:mofu/features/home/widgets/current_track_header.dart';
import 'package:mofu/features/home/widgets/playlist_sidebar.dart';
import 'package:mofu/features/home/widgets/song_table.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/widgets/playback_bar.dart';
import 'package:provider/provider.dart';
import 'package:yaru/yaru.dart';

/// 首页：侧栏播放列表 + 浏览列表摘要 + 曲目表 + 底栏。
class HomePage extends StatelessWidget {
  const HomePage({super.key});

  static const _sidebarBreakpoint = 880.0;
  static const _sidebarWidth = 220.0;

  @override
  Widget build(BuildContext context) {
    final collection = context.watch<CollectionController>();
    // 只订阅错误文案；进度 tick 不应重建整页（含子树 SongTable）。
    final playerError = context.select<PlayerController, String?>(
      (player) => player.error,
    );
    final wide = MediaQuery.sizeOf(context).width >= _sidebarBreakpoint;

    final page = Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (collection.error != null || playerError != null)
          Padding(
            padding: const EdgeInsets.fromLTRB(12, 12, 12, 0),
            child: YaruInfoBox(
              yaruInfoType: YaruInfoType.danger,
              color: StrawberryCream.strawberry,
              title: const Text('出错了'),
              subtitle: Text(
                collection.error ?? playerError!,
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
              ),
              trailing: TextButton(
                onPressed: () {
                  // 错误在下次成功操作时清除；这里触发一次刷新。
                  collection.load();
                },
                child: const Text('重试'),
              ),
            ),
          ),
        const CurrentTrackHeader(),
        const Expanded(child: SongTable()),
      ],
    );

    final body = wide
        ? YaruPanedView(
            pane: const PlaylistSidebar(),
            page: page,
            layoutDelegate: const YaruResizablePaneDelegate(
              initialPaneSize: _sidebarWidth,
              minPaneSize: 180,
              minPageSize: 360,
            ),
          )
        : page;

    return Scaffold(
      appBar: wide
          ? null
          : AppBar(
              title: const Text('Mofu'),
              actions: [
                YaruIconButton(
                  tooltip: '打开音乐目录',
                  onPressed: collection.loading
                      ? null
                      : collection.pickAndAddDirectory,
                  icon: const Icon(Icons.folder_open_rounded),
                ),
                YaruIconButton(
                  tooltip: '添加音乐文件',
                  onPressed: collection.loading
                      ? null
                      : collection.pickAndAddFiles,
                  icon: const Icon(Icons.library_add_rounded),
                ),
              ],
            ),
      drawer: wide
          ? null
          : Drawer(
              backgroundColor: Theme.of(context).colorScheme.surface,
              child: const SafeArea(child: PlaylistSidebar()),
            ),
      body: body,
      bottomNavigationBar: const PlaybackBar(),
    );
  }
}
