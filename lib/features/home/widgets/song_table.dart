import 'dart:async';

import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/collection/collection_controller.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/src/rust/api/collection.dart';
import 'package:mofu/widgets/cover_view.dart';
import 'package:mofu/widgets/duration_format.dart';
import 'package:provider/provider.dart';

/// 可滚动表格化歌曲列表。
class SongTable extends StatelessWidget {
  const SongTable({super.key});

  @override
  Widget build(BuildContext context) {
    final collection = context.watch<CollectionController>();
    // 只订阅高亮相关字段；进度 tick 不应整表重建。
    final playingPath = context.select<PlayerController, String?>(
      (player) => player.currentPath,
    );
    final player = context.read<PlayerController>();
    final playlist = collection.selectedPlaylist;
    final songs = playlist?.songs ?? const <Song>[];

    if (playlist == null) {
      return const Center(child: Text('选择或导入一个播放列表'));
    }

    if (songs.isEmpty) {
      return Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              Icons.music_off_rounded,
              size: 42,
              color: StrawberryCream.wine.withValues(alpha: 0.35),
            ),
            const SizedBox(height: 12),
            Text('这个列表还没有歌曲', style: Theme.of(context).textTheme.titleMedium),
            const SizedBox(height: 8),
            Text(
              playlist.kind == PlaylistKind.directory
                  ? '试试刷新目录，或换一个音乐文件夹'
                  : '添加音乐文件，或从其他列表导入',
              style: Theme.of(context).textTheme.bodyMedium,
            ),
            const SizedBox(height: 16),
            if (playlist.kind == PlaylistKind.virtual)
              FilledButton.icon(
                onPressed: collection.loading
                    ? null
                    : collection.pickAndAddFiles,
                icon: const Icon(Icons.library_add_rounded),
                label: const Text('添加音乐文件'),
              )
            else
              FilledButton.icon(
                onPressed: collection.loading
                    ? null
                    : () => collection.refresh(playlist.id),
                icon: const Icon(Icons.refresh_rounded),
                label: const Text('刷新目录'),
              ),
          ],
        ),
      );
    }

    final selectedPlaylist = playlist;

    _SongRow buildRow(int index) {
      final song = songs[index];
      final playing = playingPath == song.path;
      return _SongRow(
        key: ValueKey(song.path),
        index: index,
        song: song,
        playing: playing,
        onPlay: () {
          unawaited(
            player.playIndex(playlistId: selectedPlaylist.id, path: song.path),
          );
        },
        onEnqueueNext: () => player.enqueueNext(song.path),
        onOpenNowPlaying: () {
          if (!playing) {
            unawaited(
              player.playIndex(
                playlistId: selectedPlaylist.id,
                path: song.path,
              ),
            );
          }
          context.go('/now-playing');
        },
      );
    }

    return Column(
      children: [
        const _TableHeader(),
        Expanded(
          child: _FastScrollSongList(
            itemCount: songs.length,
            itemBuilder: (_, index) => buildRow(index),
          ),
        ),
      ],
    );
  }
}

class _FastScrollSongList extends StatefulWidget {
  const _FastScrollSongList({
    required this.itemCount,
    required this.itemBuilder,
  });

  final int itemCount;
  final IndexedWidgetBuilder itemBuilder;

  @override
  State<_FastScrollSongList> createState() => _FastScrollSongListState();
}

class _FastScrollSongListState extends State<_FastScrollSongList> {
  bool _isDragging = false;

  @override
  Widget build(BuildContext context) {
    return NotificationListener<ScrollNotification>(
      onNotification: _handleScrollNotification,
      child: ExcludeSemantics(
        excluding: _isDragging,
        child: ListView.builder(
          padding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
          // 每行高度为上下 padding 16 加封面高度 40。
          itemExtent: 56,
          itemCount: widget.itemCount,
          itemBuilder: widget.itemBuilder,
        ),
      ),
    );
  }

  bool _handleScrollNotification(ScrollNotification notification) {
    final isDragging = switch (notification) {
      ScrollStartNotification(:final dragDetails) => dragDetails != null,
      ScrollUpdateNotification(:final dragDetails) =>
        dragDetails != null ? true : _isDragging,
      ScrollEndNotification() => false,
      _ => _isDragging,
    };
    if (isDragging != _isDragging) {
      setState(() => _isDragging = isDragging);
    }
    return false;
  }
}

class _TableHeader extends StatelessWidget {
  const _TableHeader();

  @override
  Widget build(BuildContext context) {
    final style = Theme.of(
      context,
    ).textTheme.labelLarge?.copyWith(color: StrawberryCream.wineMuted);
    return Padding(
      padding: const EdgeInsets.fromLTRB(28, 8, 28, 4),
      child: Row(
        children: [
          SizedBox(width: 36, child: Text('#', style: style)),
          const SizedBox(width: 52),
          Expanded(flex: 3, child: Text('歌曲', style: style)),
          Expanded(flex: 2, child: Text('专辑', style: style)),
          Expanded(flex: 2, child: Text('歌手', style: style)),
          SizedBox(
            width: 64,
            child: Text('时长', style: style, textAlign: TextAlign.end),
          ),
        ],
      ),
    );
  }
}

class _SongRow extends StatelessWidget {
  const _SongRow({
    super.key,
    required this.index,
    required this.song,
    required this.playing,
    required this.onPlay,
    required this.onEnqueueNext,
    required this.onOpenNowPlaying,
  });

  final int index;
  final Song song;
  final bool playing;
  final VoidCallback onPlay;
  final VoidCallback onEnqueueNext;
  final VoidCallback onOpenNowPlaying;

  @override
  Widget build(BuildContext context) {
    final meta = song.metadata;
    final title = song.displayTitle;
    final album = meta.album?.trim().isNotEmpty == true ? meta.album! : '—';
    final artist = meta.artist?.trim().isNotEmpty == true ? meta.artist! : '—';

    return Material(
      color: playing
          ? StrawberryCream.strawberry.withValues(alpha: 0.12)
          : Colors.transparent,
      borderRadius: BorderRadius.circular(StrawberryCream.radiusSm),
      child: InkWell(
        borderRadius: BorderRadius.circular(StrawberryCream.radiusSm),
        onTap: onPlay,
        onDoubleTap: onOpenNowPlaying,
        onSecondaryTapDown: (details) =>
            _showContextMenu(context, details.globalPosition),
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
          child: Row(
            children: [
              SizedBox(
                width: 36,
                child: playing
                    ? const Align(
                        alignment: Alignment.centerLeft,
                        child: Icon(
                          Icons.graphic_eq_rounded,
                          size: 18,
                          color: StrawberryCream.strawberry,
                        ),
                      )
                    : Text(
                        '${index + 1}',
                        style: Theme.of(context).textTheme.bodyMedium,
                      ),
              ),
              GestureDetector(
                onTap: onOpenNowPlaying,
                child: CoverView(cover: meta.cover, size: 40),
              ),
              const SizedBox(width: 12),
              Expanded(
                flex: 3,
                child: Text(
                  title,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: Theme.of(context).textTheme.titleMedium?.copyWith(
                    color: playing
                        ? StrawberryCream.strawberry
                        : StrawberryCream.wine,
                  ),
                ),
              ),
              Expanded(
                flex: 2,
                child: Text(
                  album,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: Theme.of(context).textTheme.bodyMedium,
                ),
              ),
              Expanded(
                flex: 2,
                child: Text(
                  artist,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: Theme.of(context).textTheme.bodyMedium,
                ),
              ),
              SizedBox(
                width: 64,
                child: Text(
                  DurationFormat.fromMs(song.durationMs),
                  textAlign: TextAlign.end,
                  style: Theme.of(context).textTheme.bodyMedium,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }

  /// 右键时在指针位置弹出操作菜单；选中后由回调驱动播放控制。
  Future<void> _showContextMenu(BuildContext context, Offset position) async {
    final overlay =
        Overlay.of(context).context.findRenderObject()! as RenderBox;
    final playNext = await showMenu<bool>(
      context: context,
      position: RelativeRect.fromLTRB(
        position.dx,
        position.dy,
        overlay.size.width - position.dx,
        overlay.size.height - position.dy,
      ),
      items: const [PopupMenuItem(value: true, child: Text('下一首播放'))],
    );
    if (playNext == true) {
      onEnqueueNext();
    }
  }
}
