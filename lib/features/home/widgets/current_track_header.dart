import 'package:flutter/material.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/collection/collection_controller.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/src/rust/api/collection.dart';
import 'package:mofu/widgets/cover_view.dart';
import 'package:provider/provider.dart';

/// 主区域顶部：当前浏览列表的摘要（封面、名称、曲目数）。
///
/// 正在播放的信息只出现在底部 [PlaybackBar] 与播放详情页，避免与列表浏览混淆。
class CurrentTrackHeader extends StatelessWidget {
  const CurrentTrackHeader({super.key});

  @override
  Widget build(BuildContext context) {
    final collection = context.watch<CollectionController>();
    // 仅需调用 playAll，不订阅播放进度等无关状态。
    final player = context.read<PlayerController>();
    final playlist = collection.selectedPlaylist;

    if (collection.playlists.isEmpty) {
      return _EmptyCollectionActions(
        onOpenDirectory: collection.pickAndAddDirectory,
        onAddFiles: collection.pickAndAddFiles,
        loading: collection.loading,
      );
    }

    if (playlist == null) {
      return const Padding(
        padding: EdgeInsets.fromLTRB(24, 28, 24, 12),
        child: Text('选择或导入一个播放列表'),
      );
    }

    final cover = playlist.cover;
    final kindLabel = switch (playlist.kind) {
      PlaylistKind.directory => '目录',
      PlaylistKind.virtual => '虚拟列表',
    };
    final count = playlist.songs.length;
    final subtitleParts = <String>[
      '$count 首',
      if (playlist.kind == PlaylistKind.directory &&
          playlist.directoryPath != null &&
          playlist.directoryPath!.isNotEmpty)
        playlist.directoryPath!,
    ];
    final subtitle = subtitleParts.join(' · ');

    return Padding(
      padding: const EdgeInsets.fromLTRB(24, 20, 24, 8),
      child: Material(
        color: StrawberryCream.card,
        borderRadius: BorderRadius.circular(StrawberryCream.radius),
        child: Padding(
          padding: const EdgeInsets.all(16),
          child: Row(
            children: [
              CoverView(cover: cover, size: 96, shadow: true),
              const SizedBox(width: 20),
              Expanded(
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    Text(
                      kindLabel,
                      style: Theme.of(context).textTheme.bodyMedium,
                    ),
                    const SizedBox(height: 4),
                    Text(
                      playlist.name,
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style: Theme.of(context).textTheme.headlineMedium,
                    ),
                    const SizedBox(height: 6),
                    Text(
                      subtitle,
                      maxLines: 2,
                      overflow: TextOverflow.ellipsis,
                      style: Theme.of(context).textTheme.bodyMedium,
                    ),
                  ],
                ),
              ),
              const SizedBox(width: 12),
              FilledButton.icon(
                onPressed: playlist.songs.isEmpty
                    ? null
                    : () => player.playAll(playlistId: playlist.id),
                icon: const Icon(Icons.play_arrow_rounded),
                label: const Text('播放全部'),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class _EmptyCollectionActions extends StatelessWidget {
  const _EmptyCollectionActions({
    required this.onOpenDirectory,
    required this.onAddFiles,
    required this.loading,
  });

  final VoidCallback onOpenDirectory;
  final VoidCallback onAddFiles;
  final bool loading;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(24, 28, 24, 12),
      child: DecoratedBox(
        decoration: BoxDecoration(
          gradient: StrawberryCream.coverPlaceholder,
          borderRadius: BorderRadius.circular(StrawberryCream.radius),
          boxShadow: StrawberryCream.softShadow(blur: 20, dy: 8),
        ),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 28, vertical: 32),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                '把音乐放进奶油里',
                style: Theme.of(context).textTheme.headlineMedium,
              ),
              const SizedBox(height: 8),
              Text(
                '打开本地目录或添加音乐文件，开始构建你的曲库。',
                style: Theme.of(context).textTheme.bodyMedium,
              ),
              const SizedBox(height: 20),
              Wrap(
                spacing: 12,
                runSpacing: 12,
                children: [
                  FilledButton.icon(
                    onPressed: loading ? null : onOpenDirectory,
                    icon: const Icon(Icons.folder_open_rounded),
                    label: const Text('打开音乐目录'),
                  ),
                  OutlinedButton.icon(
                    onPressed: loading ? null : onAddFiles,
                    icon: const Icon(Icons.library_add_rounded),
                    label: const Text('添加音乐文件'),
                  ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}
