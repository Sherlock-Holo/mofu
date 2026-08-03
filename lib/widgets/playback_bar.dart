import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/src/rust/api/player.dart';
import 'package:mofu/widgets/cover_view.dart';
import 'package:mofu/widgets/duration_format.dart';
import 'package:mofu/widgets/playback_mode_button.dart';
import 'package:provider/provider.dart';

/// 底部播放控制条：进度、跳转、音量与模式。
class PlaybackBar extends StatelessWidget {
  const PlaybackBar({
    super.key,
    this.showNowPlayingEntry = true,
    this.compact = false,
  });

  final bool showNowPlayingEntry;
  final bool compact;

  @override
  Widget build(BuildContext context) {
    final player = context.watch<PlayerController>();
    final snapshot = player.snapshot;
    final meta = snapshot?.metadata;
    final title = meta?.title?.trim().isNotEmpty == true
        ? meta!.title!
        : _fallbackTitle(snapshot);
    final artist =
        meta?.artist?.trim().isNotEmpty == true ? meta!.artist! : '未知歌手';

    return Material(
      color: StrawberryCream.card,
      elevation: 0,
      child: Container(
        decoration: BoxDecoration(
          border: Border(
            top: BorderSide(
              color: StrawberryCream.strawberry.withValues(alpha: 0.12),
            ),
          ),
          boxShadow: [
            BoxShadow(
              color: StrawberryCream.strawberry.withValues(alpha: 0.08),
              blurRadius: 18,
              offset: const Offset(0, -4),
            ),
          ],
        ),
        padding: EdgeInsets.fromLTRB(
          compact ? 12 : 20,
          10,
          compact ? 12 : 20,
          12,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            _ProgressSlider(player: player),
            const SizedBox(height: 4),
            Row(
              children: [
                Expanded(
                  child: Align(
                    alignment: Alignment.centerLeft,
                    child: showNowPlayingEntry
                        ? InkWell(
                            borderRadius: BorderRadius.circular(
                              StrawberryCream.radiusSm,
                            ),
                            onTap:
                                meta == null && snapshot?.currentIndex == null
                                    ? null
                                    : () => context.go('/now-playing'),
                            child: Row(
                              mainAxisSize: MainAxisSize.min,
                              children: [
                                CoverView(
                                  cover: meta?.cover,
                                  size: compact ? 44 : 52,
                                ),
                                const SizedBox(width: 12),
                                Flexible(
                                  child: Column(
                                    crossAxisAlignment:
                                        CrossAxisAlignment.start,
                                    children: [
                                      Text(
                                        title,
                                        maxLines: 1,
                                        overflow: TextOverflow.ellipsis,
                                        style: Theme.of(context)
                                            .textTheme
                                            .titleMedium,
                                      ),
                                      Text(
                                        artist,
                                        maxLines: 1,
                                        overflow: TextOverflow.ellipsis,
                                        style: Theme.of(context)
                                            .textTheme
                                            .bodyMedium,
                                      ),
                                    ],
                                  ),
                                ),
                              ],
                            ),
                          )
                        : const SizedBox.shrink(),
                  ),
                ),
                Row(
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    IconButton(
                      tooltip: '上一首',
                      onPressed: player.previous,
                      icon: const Icon(Icons.skip_previous_rounded),
                    ),
                    IconButton(
                      tooltip: '后退 10 秒',
                      onPressed: () => player.seekRelative(-10000),
                      icon: const Icon(Icons.replay_10_rounded),
                    ),
                    _PlayPauseButton(
                      playing: player.isPlaying,
                      onPressed: player.togglePlayPause,
                    ),
                    IconButton(
                      tooltip: '前进 10 秒',
                      onPressed: () => player.seekRelative(10000),
                      icon: const Icon(Icons.forward_10_rounded),
                    ),
                    IconButton(
                      tooltip: '下一首',
                      onPressed: player.next,
                      icon: const Icon(Icons.skip_next_rounded),
                    ),
                  ],
                ),
                Expanded(
                  child: Align(
                    alignment: Alignment.centerRight,
                    child: Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        if (compact)
                          _CompactVolumeButton(
                            volume: player.volume,
                            onChanged: player.setVolume,
                          )
                        else ...[
                          _VolumeIcon(volume: player.volume, size: 20),
                          _VolumeLabel(volume: player.volume),
                          SizedBox(
                            width: 108,
                            child: Slider(
                              value: player.volume.clamp(0, 1),
                              onChanged: player.setVolume,
                            ),
                          ),
                        ],
                        PlaybackModeButton(
                          mode: player.playbackMode,
                          onChanged: player.setPlaybackMode,
                        ),
                      ],
                    ),
                  ),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }

  String _fallbackTitle(PlayerSnapshot? snapshot) {
    final index = snapshot?.currentIndex;
    if (index == null || snapshot == null) {
      return '未在播放';
    }
    for (final item in snapshot.queue) {
      if (item.index == index) {
        final path = item.path;
        final slash = path.replaceAll('\\', '/').split('/').last;
        return slash.isEmpty ? '未知曲目' : slash;
      }
    }
    return '未知曲目';
  }
}

/// 拖动时本地预览，松手后再 seek，避免拖动中反复打 FFI。
class _ProgressSlider extends StatefulWidget {
  const _ProgressSlider({required this.player});

  final PlayerController player;

  @override
  State<_ProgressSlider> createState() => _ProgressSliderState();
}

class _ProgressSliderState extends State<_ProgressSlider> {
  double? _scrubbing;

  @override
  Widget build(BuildContext context) {
    final player = widget.player;
    final position = player.positionMs.toInt().toDouble();
    final duration =
        player.durationMs.toInt().toDouble().clamp(1, double.infinity);
    final live = (position / duration).clamp(0.0, 1.0);
    final progress = _scrubbing ?? (live.isFinite ? live : 0.0);
    final displayMs = BigInt.from((progress * duration).round());

    return Row(
      children: [
        Text(
          DurationFormat.fromMs(displayMs),
          style: Theme.of(context).textTheme.bodySmall?.copyWith(
                color: StrawberryCream.wineMuted,
              ),
        ),
        Expanded(
          child: Slider(
            value: progress,
            onChanged: (value) => setState(() => _scrubbing = value),
            onChangeEnd: (value) {
              final ms = (value * duration).round();
              player.seek(BigInt.from(ms));
              setState(() => _scrubbing = null);
            },
          ),
        ),
        Text(
          DurationFormat.fromMs(player.durationMs),
          style: Theme.of(context).textTheme.bodySmall?.copyWith(
                color: StrawberryCream.wineMuted,
              ),
        ),
      ],
    );
  }
}

class _CompactVolumeButton extends StatelessWidget {
  const _CompactVolumeButton({
    required this.volume,
    required this.onChanged,
  });

  final double volume;
  final ValueChanged<double> onChanged;

  @override
  Widget build(BuildContext context) {
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        IconButton(
          tooltip: '音量',
          onPressed: () => _showVolumeSheet(context),
          icon: _VolumeIcon(
            volume: volume,
            color: StrawberryCream.wine,
          ),
        ),
        _VolumeLabel(volume: volume),
      ],
    );
  }

  Future<void> _showVolumeSheet(BuildContext context) async {
    var current = volume.clamp(0.0, 1.0);
    await showModalBottomSheet<void>(
      context: context,
      backgroundColor: StrawberryCream.cream,
      builder: (context) {
        return SafeArea(
          child: Padding(
            padding: const EdgeInsets.fromLTRB(20, 16, 20, 24),
            child: StatefulBuilder(
              builder: (context, setModalState) {
                return Row(
                  children: [
                    _VolumeIcon(volume: current),
                    _VolumeLabel(volume: current),
                    Expanded(
                      child: Slider(
                        value: current,
                        onChanged: (value) {
                          setModalState(() => current = value);
                          onChanged(value);
                        },
                      ),
                    ),
                  ],
                );
              },
            ),
          ),
        );
      },
    );
  }
}

class _VolumeIcon extends StatelessWidget {
  const _VolumeIcon({
    required this.volume,
    this.size,
    this.color,
  });

  final double volume;
  final double? size;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final normalized = volume.clamp(0.0, 1.0);
    final icon = switch (normalized) {
      <= 0.001 => Icons.volume_off_rounded,
      <= 0.33 => Icons.volume_mute_rounded,
      <= 0.66 => Icons.volume_down_rounded,
      _ => Icons.volume_up_rounded,
    };
    return Icon(icon, size: size, color: color);
  }
}

class _VolumeLabel extends StatelessWidget {
  const _VolumeLabel({required this.volume});

  final double volume;

  @override
  Widget build(BuildContext context) {
    final percentage = (volume.clamp(0.0, 1.0) * 100).round();
    return SizedBox(
      width: 28,
      child: Text(
        '$percentage',
        textAlign: TextAlign.right,
        style: Theme.of(context).textTheme.bodySmall?.copyWith(
          color: StrawberryCream.wineMuted,
          fontFeatures: const [FontFeature.tabularFigures()],
        ),
      ),
    );
  }
}

class _PlayPauseButton extends StatelessWidget {
  const _PlayPauseButton({
    required this.playing,
    required this.onPressed,
  });

  final bool playing;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    return Material(
      color: StrawberryCream.strawberry,
      shape: const CircleBorder(),
      elevation: 0,
      shadowColor: StrawberryCream.strawberry.withValues(alpha: 0.4),
      child: InkWell(
        customBorder: const CircleBorder(),
        onTap: onPressed,
        child: Padding(
          padding: const EdgeInsets.all(12),
          child: Icon(
            playing ? Icons.pause_rounded : Icons.play_arrow_rounded,
            color: Colors.white,
            size: 28,
          ),
        ),
      ),
    );
  }
}
