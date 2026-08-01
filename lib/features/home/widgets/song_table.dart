import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/library/library_controller.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/src/rust/api/library.dart';
import 'package:mofu/widgets/cover_view.dart';
import 'package:mofu/widgets/duration_format.dart';
import 'package:provider/provider.dart';

/// 可滚动表格化歌曲列表。
class SongTable extends StatefulWidget {
  const SongTable({super.key});

  @override
  State<SongTable> createState() => _SongTableState();
}

class _SongTableState extends State<SongTable> {
  List<Song>? _cachedSongs;
  List<String> _cachedPaths = const [];

  List<String> _pathsOf(List<Song> songs) {
    if (identical(_cachedSongs, songs)) {
      return _cachedPaths;
    }
    _cachedSongs = songs;
    _cachedPaths = [for (final song in songs) song.path];
    return _cachedPaths;
  }

  @override
  Widget build(BuildContext context) {
    final library = context.watch<LibraryController>();
    // 只订阅高亮相关字段；进度 tick 不应整表重建。
    final playingPath = context.select<PlayerController, String?>(
      (player) => player.currentPath,
    );
    final player = context.read<PlayerController>();
    final playlist = library.selectedPlaylist;
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
            Text(
              '这个列表还没有歌曲',
              style: Theme.of(context).textTheme.titleMedium,
            ),
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
                onPressed: library.loading ? null : library.pickAndAddFiles,
                icon: const Icon(Icons.library_add_rounded),
                label: const Text('添加音乐文件'),
              )
            else
              FilledButton.icon(
                onPressed:
                    library.loading ? null : () => library.refresh(playlist.id),
                icon: const Icon(Icons.refresh_rounded),
                label: const Text('刷新目录'),
              ),
          ],
        ),
      );
    }

    final paths = _pathsOf(songs);

    return Column(
      children: [
        const _TableHeader(),
        Expanded(
          child: ListView.builder(
            padding: const EdgeInsets.fromLTRB(16, 0, 16, 16),
            itemCount: songs.length,
            itemBuilder: (context, index) {
              final song = songs[index];
              final playing = playingPath == song.path;
              return _SongRow(
                index: index,
                song: song,
                playing: playing,
                onPlay: () {
                  player.playPaths(
                    paths,
                    startIndex: index,
                    playlistId: playlist.id,
                  );
                },
                onOpenNowPlaying: () {
                  if (!playing) {
                    player.playPaths(
                      paths,
                      startIndex: index,
                      playlistId: playlist.id,
                    );
                  }
                  context.go('/now-playing');
                },
              );
            },
          ),
        ),
      ],
    );
  }
}

class _TableHeader extends StatelessWidget {
  const _TableHeader();

  @override
  Widget build(BuildContext context) {
    final style = Theme.of(context).textTheme.labelLarge?.copyWith(
          color: StrawberryCream.wineMuted,
        );
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
    required this.index,
    required this.song,
    required this.playing,
    required this.onPlay,
    required this.onOpenNowPlaying,
  });

  final int index;
  final Song song;
  final bool playing;
  final VoidCallback onPlay;
  final VoidCallback onOpenNowPlaying;

  @override
  Widget build(BuildContext context) {
    final meta = song.metadata;
    final title = (meta.title?.trim().isNotEmpty == true)
        ? meta.title!
        : _fileName(song.path);
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

  String _fileName(String path) {
    final slash = path.replaceAll('\\', '/').split('/').last;
    return slash.isEmpty ? '未知曲目' : slash;
  }
}
