import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/features/now_playing/widgets/lyrics_panel.dart';
import 'package:mofu/features/now_playing/widgets/rotating_cover.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/settings/settings_controller.dart';
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
    // path + cover 身份：元数据晚到时也要重建；progress tick 保留同一 cover 引用则跳过。
    final key = context.select<PlayerController, (String?, int)>((player) {
      return (player.currentPath, identityHashCode(player.metadata?.cover));
    });
    final cover = context.read<PlayerController>().metadata?.cover;
    return BlurCoverBackground(key: ValueKey(key), cover: cover, child: child);
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
    // 须订阅文案与封面身份：切歌后 path 先到、metadata 后到时仍能刷新；
    // progress tick 下这些字段稳定，不会拖累整列重建。
    final track = context
        .select<
          PlayerController,
          ({
            String? path,
            bool playing,
            String title,
            String artist,
            String album,
            int coverId,
          })
        >((player) {
          return (
            path: player.currentPath,
            playing: player.isPlaying,
            title: player.uiTitle,
            artist: player.uiArtist,
            album: player.uiAlbum,
            coverId: identityHashCode(player.metadata?.cover),
          );
        });
    final cover = context.read<PlayerController>().metadata?.cover;
    final pauseWhenInactive = context.select<SettingsController, bool>(
      (settings) => settings.pauseCoverAnimationInBackground,
    );

    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        RotatingCover(
          key: ValueKey(track.path),
          cover: cover,
          playing: track.playing,
          pauseWhenInactive: pauseWhenInactive,
          size: size,
        ),
        const SizedBox(height: 28),
        Text(
          track.title,
          textAlign: TextAlign.center,
          maxLines: 2,
          overflow: TextOverflow.ellipsis,
          style: Theme.of(context).textTheme.headlineMedium,
        ),
        const SizedBox(height: 8),
        Text(
          track.album,
          textAlign: TextAlign.center,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: Theme.of(
            context,
          ).textTheme.bodyLarge?.copyWith(color: StrawberryCream.wineMuted),
        ),
        const SizedBox(height: 4),
        Text(
          track.artist,
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
    // 歌词列表身份随 full metadata 到达而变；activeLine 驱动高亮。
    final cursor = context.select<PlayerController, (String?, int, BigInt?)>((
      player,
    ) {
      return (
        player.currentPath,
        identityHashCode(player.metadata?.lyrics),
        player.activeLyricsLine,
      );
    });
    final lyrics =
        context.read<PlayerController>().metadata?.lyrics ?? const <Lyrics>[];

    return LyricsPanel(
      key: ValueKey(cursor.$1),
      lyrics: lyrics,
      activeLineHint: cursor.$3,
    );
  }
}
