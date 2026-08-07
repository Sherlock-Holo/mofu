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
///
/// 进度 tick 只驱动歌词行高亮与底栏进度；背景 / 封面 / 文案按曲目变化订阅，避免整页重建。
class NowPlayingPage extends StatelessWidget {
  const NowPlayingPage({super.key});

  static const _wideBreakpoint = 960.0;

  @override
  Widget build(BuildContext context) {
    final wide = MediaQuery.sizeOf(context).width >= _wideBreakpoint;

    return Scaffold(
      body: _NowPlayingBackground(
        child: SafeArea(
          bottom: false,
          child: Column(
            children: [
              const _NowPlayingAppBar(),
              const Expanded(child: _NowPlayingBody()),
              PlaybackBar(showNowPlayingEntry: false, compact: !wide),
            ],
          ),
        ),
      ),
    );
  }
}

class _NowPlayingAppBar extends StatelessWidget {
  const _NowPlayingAppBar();

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(8, 4, 16, 0),
      child: Row(
        children: [
          IconButton(
            tooltip: '返回首页',
            onPressed: () => context.go('/'),
            icon: const Icon(Icons.arrow_back_rounded),
          ),
          Expanded(
            child: Text('正在播放', style: Theme.of(context).textTheme.titleMedium),
          ),
        ],
      ),
    );
  }
}

class _NowPlayingBackground extends StatelessWidget {
  const _NowPlayingBackground({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    // 仅在曲目切换时重建模糊层；进度 tick 的 path 不变则跳过。
    final path = context.select<PlayerController, String?>(
      (player) => player.currentPath,
    );
    final cover = context.read<PlayerController>().metadata?.cover;
    return BlurCoverBackground(key: ValueKey(path), cover: cover, child: child);
  }
}

class _NowPlayingBody extends StatelessWidget {
  const _NowPlayingBody();

  static const _wideBreakpoint = 960.0;

  @override
  Widget build(BuildContext context) {
    final wide = MediaQuery.sizeOf(context).width >= _wideBreakpoint;
    return Padding(
      padding: EdgeInsets.symmetric(horizontal: wide ? 40 : 20, vertical: 12),
      child: wide
          ? const Row(
              children: [
                Expanded(flex: 5, child: _CoverColumn(size: 300)),
                SizedBox(width: 36),
                Expanded(flex: 6, child: _LyricsHost()),
              ],
            )
          : const Column(
              children: [
                _CoverColumn(size: 220),
                SizedBox(height: 16),
                Expanded(child: _LyricsHost()),
              ],
            ),
    );
  }
}

class _CoverColumn extends StatelessWidget {
  const _CoverColumn({required this.size});

  final double size;

  @override
  Widget build(BuildContext context) {
    final path = context.select<PlayerController, String?>(
      (player) => player.currentPath,
    );
    final playing = context.select<PlayerController, bool>(
      (player) => player.isPlaying,
    );
    final player = context.read<PlayerController>();
    final meta = player.metadata;
    final title = (player.displayTitle?.trim().isNotEmpty == true)
        ? player.displayTitle!
        : (meta?.title?.trim().isNotEmpty == true ? meta!.title! : '未在播放');
    final artist = meta?.artist?.trim().isNotEmpty == true
        ? meta!.artist!
        : '未知歌手';
    final album = meta?.album?.trim().isNotEmpty == true
        ? meta!.album!
        : '未知专辑';

    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        RotatingCover(
          key: ValueKey(path),
          cover: meta?.cover,
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
          style: Theme.of(
            context,
          ).textTheme.bodyLarge?.copyWith(color: StrawberryCream.wineMuted),
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

class _LyricsHost extends StatelessWidget {
  const _LyricsHost();

  @override
  Widget build(BuildContext context) {
    final path = context.select<PlayerController, String?>(
      (player) => player.currentPath,
    );
    final activeLine = context.select<PlayerController, BigInt?>(
      (player) => player.activeLyricsLine,
    );
    final lyrics =
        context.read<PlayerController>().metadata?.lyrics ?? const <Lyrics>[];

    return LyricsPanel(
      key: ValueKey(path),
      lyrics: lyrics,
      activeLineHint: activeLine,
    );
  }
}
