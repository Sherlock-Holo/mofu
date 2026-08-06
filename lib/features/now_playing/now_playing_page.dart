import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/features/now_playing/widgets/lyrics_panel.dart';
import 'package:mofu/features/now_playing/widgets/rotating_cover.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/src/rust/api/player.dart';
import 'package:mofu/widgets/cover_view.dart';
import 'package:mofu/widgets/playback_bar.dart';
import 'package:provider/provider.dart';

/// 沉浸式播放页：模糊封面背景、旋转圆封面、同步歌词、复用底栏。
class NowPlayingPage extends StatelessWidget {
  const NowPlayingPage({super.key});

  static const _wideBreakpoint = 960.0;

  @override
  Widget build(BuildContext context) {
    final player = context.watch<PlayerController>();
    final meta = player.metadata;
    final wide = MediaQuery.sizeOf(context).width >= _wideBreakpoint;

    final title = (meta?.title?.trim().isNotEmpty == true)
        ? meta!.title!
        : _fallbackTitle(player);
    final artist =
        meta?.artist?.trim().isNotEmpty == true ? meta!.artist! : '未知歌手';
    final album =
        meta?.album?.trim().isNotEmpty == true ? meta!.album! : '未知专辑';

    return Scaffold(
      body: BlurCoverBackground(
        cover: meta?.cover,
        child: SafeArea(
          bottom: false,
          child: Column(
            children: [
              Padding(
                padding: const EdgeInsets.fromLTRB(8, 4, 16, 0),
                child: Row(
                  children: [
                    IconButton(
                      tooltip: '返回首页',
                      onPressed: () => context.go('/'),
                      icon: const Icon(Icons.arrow_back_rounded),
                    ),
                    Expanded(
                      child: Text(
                        '正在播放',
                        style: Theme.of(context).textTheme.titleMedium,
                      ),
                    ),
                  ],
                ),
              ),
              Expanded(
                child: Padding(
                  padding: EdgeInsets.symmetric(
                    horizontal: wide ? 40 : 20,
                    vertical: 12,
                  ),
                  child: wide
                      ? Row(
                          children: [
                            Expanded(
                              flex: 5,
                              child: _CoverColumn(
                                cover: meta?.cover,
                                playing: player.isPlaying,
                                title: title,
                                album: album,
                                artist: artist,
                                size: 300,
                              ),
                            ),
                            const SizedBox(width: 36),
                            Expanded(
                              flex: 6,
                              child: LyricsPanel(
                                lyrics: meta?.lyrics ?? const [],
                                positionMs: player.positionMs,
                              ),
                            ),
                          ],
                        )
                      : Column(
                          children: [
                            _CoverColumn(
                              cover: meta?.cover,
                              playing: player.isPlaying,
                              title: title,
                              album: album,
                              artist: artist,
                              size: 220,
                            ),
                            const SizedBox(height: 16),
                            Expanded(
                              child: LyricsPanel(
                                lyrics: meta?.lyrics ?? const [],
                                positionMs: player.positionMs,
                              ),
                            ),
                          ],
                        ),
                ),
              ),
              PlaybackBar(
                showNowPlayingEntry: false,
                compact: !wide,
              ),
            ],
          ),
        ),
      ),
    );
  }

  String _fallbackTitle(PlayerController player) {
    final path = player.currentPath;
    if (path == null) {
      return '未在播放';
    }
    final name = path.replaceAll('\\', '/').split('/').last;
    return name.isEmpty ? '未知曲目' : name;
  }
}

class _CoverColumn extends StatelessWidget {
  const _CoverColumn({
    required this.cover,
    required this.playing,
    required this.title,
    required this.album,
    required this.artist,
    required this.size,
  });

  final CoverArt? cover;
  final bool playing;
  final String title;
  final String album;
  final String artist;
  final double size;

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        RotatingCover(
          cover: cover,
          playing: playing,
          size: size,
        ),
        const SizedBox(height: 28),
        Text(
          title,
          textAlign: TextAlign.center,
          maxLines: 2,
          overflow: TextOverflow.ellipsis,
          style: Theme.of(context).textTheme.headlineMedium,
        ),
        const SizedBox(height: 8),
        Text(
          album,
          textAlign: TextAlign.center,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: Theme.of(context).textTheme.bodyLarge?.copyWith(
                color: StrawberryCream.wineMuted,
              ),
        ),
        const SizedBox(height: 4),
        Text(
          artist,
          textAlign: TextAlign.center,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: Theme.of(context).textTheme.bodyMedium,
        ),
      ],
    );
  }
}
