import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/library/library_controller.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/src/rust/api/player.dart';
import 'package:mofu/widgets/cover_view.dart';
import 'package:provider/provider.dart';

/// 主区域顶部：当前歌曲封面与元数据；空曲库时提供导入入口。
class CurrentTrackHeader extends StatelessWidget {
  const CurrentTrackHeader({super.key});

  @override
  Widget build(BuildContext context) {
    final library = context.watch<LibraryController>();
    final player = context.watch<PlayerController>();
    final meta = player.metadata;
    final playlist = library.selectedPlaylist;

    if (library.playlists.isEmpty ||
        library.playlists.every((p) => p.songs.isEmpty)) {
      return _EmptyLibraryActions(
        onOpenDirectory: library.pickAndAddDirectory,
        onAddFiles: library.pickAndAddFiles,
        loading: library.loading,
      );
    }

    final title = _titleOf(meta, player);
    final artist = _orUnknown(meta?.artist, '未知歌手');
    final album = _orUnknown(meta?.album, '未知专辑');
    final year = meta?.year;

    return Padding(
      padding: const EdgeInsets.fromLTRB(24, 20, 24, 8),
      child: Material(
        color: StrawberryCream.card,
        borderRadius: BorderRadius.circular(StrawberryCream.radius),
        child: InkWell(
          borderRadius: BorderRadius.circular(StrawberryCream.radius),
          onTap: meta == null && player.currentIndex == null
              ? null
              : () => context.go('/now-playing'),
          child: Padding(
            padding: const EdgeInsets.all(16),
            child: Row(
              children: [
                CoverView(
                  cover: meta?.cover,
                  size: 96,
                  shadow: true,
                ),
                const SizedBox(width: 20),
                Expanded(
                  child: Column(
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        playlist?.name ?? '曲库',
                        style: Theme.of(context).textTheme.bodyMedium,
                      ),
                      const SizedBox(height: 4),
                      Text(
                        title,
                        maxLines: 2,
                        overflow: TextOverflow.ellipsis,
                        style: Theme.of(context).textTheme.headlineMedium,
                      ),
                      const SizedBox(height: 6),
                      Text(
                        [
                          artist,
                          album,
                          if (year != null) '$year',
                        ].join(' · '),
                        maxLines: 2,
                        overflow: TextOverflow.ellipsis,
                        style: Theme.of(context).textTheme.bodyMedium,
                      ),
                    ],
                  ),
                ),
                Icon(
                  Icons.chevron_right_rounded,
                  color: StrawberryCream.wine.withValues(alpha: 0.35),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  String _titleOf(TrackMetadata? meta, PlayerController player) {
    final titled = meta?.title?.trim();
    if (titled != null && titled.isNotEmpty) {
      return titled;
    }
    final index = player.currentIndex;
    if (index != null) {
      for (final item in player.queue) {
        if (item.index == index) {
          return _fileName(item.path);
        }
      }
    }
    return '选择一首歌曲开始播放';
  }

  String _orUnknown(String? value, String fallback) {
    final trimmed = value?.trim();
    if (trimmed == null || trimmed.isEmpty) {
      return fallback;
    }
    return trimmed;
  }

  String _fileName(String path) {
    final slash = path.replaceAll('\\', '/').split('/').last;
    return slash.isEmpty ? '未知曲目' : slash;
  }
}

class _EmptyLibraryActions extends StatelessWidget {
  const _EmptyLibraryActions({
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
