import 'package:flutter/material.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/src/rust/api/player.dart';

/// 同步/静态歌词面板。
class LyricsPanel extends StatefulWidget {
  const LyricsPanel({
    super.key,
    required this.lyrics,
    this.positionMs,
    this.activeLineHint,
    this.onDownloadLyrics,
  });

  final List<Lyrics> lyrics;

  /// Fallback when [activeLineHint] is null (e.g. unit tests).
  final BigInt? positionMs;

  /// Prefers the Rust-computed active line so the host can skip position ticks.
  final BigInt? activeLineHint;

  /// Empty-state entry to start a lyrics search. Omitted when null.
  final VoidCallback? onDownloadLyrics;

  @override
  State<LyricsPanel> createState() => _LyricsPanelState();
}

class _LyricsPanelState extends State<LyricsPanel> {
  final _scrollController = ScrollController();
  int? _lastActiveIndex;

  Lyrics? get _primary {
    if (widget.lyrics.isEmpty) {
      return null;
    }
    for (final item in widget.lyrics) {
      if (item.content.kind == LyricsContentKind.timed &&
          (item.content.lines?.isNotEmpty ?? false)) {
        return item;
      }
    }
    return widget.lyrics.first;
  }

  @override
  void didUpdateWidget(covariant LyricsPanel oldWidget) {
    super.didUpdateWidget(oldWidget);
    final lines = _timedLines;
    if (lines == null) {
      return;
    }
    final active = _activeIndex(lines);
    if (active != _lastActiveIndex) {
      _lastActiveIndex = active;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        _scrollToActive(active, lines.length);
      });
    }
  }

  List<LyricsLine>? get _timedLines {
    final primary = _primary;
    if (primary == null) {
      return null;
    }
    if (primary.content.kind != LyricsContentKind.timed) {
      return null;
    }
    final lines = primary.content.lines;
    if (lines == null || lines.isEmpty) {
      return null;
    }
    return lines;
  }

  int _activeIndex(List<LyricsLine> lines) {
    final hint = widget.activeLineHint;
    if (hint != null) {
      final index = hint.toInt();
      if (index >= 0 && index < lines.length) {
        return index;
      }
    }
    final pos = widget.positionMs ?? BigInt.zero;
    var active = 0;
    for (var i = 0; i < lines.length; i++) {
      if (lines[i].startMs <= pos) {
        active = i;
      } else {
        break;
      }
    }
    return active;
  }

  void _scrollToActive(int active, int total) {
    if (!_scrollController.hasClients || total == 0) {
      return;
    }
    const itemExtent = 44.0;
    final viewport = _scrollController.position.viewportDimension;
    final target = (active * itemExtent) - (viewport / 2) + (itemExtent / 2);
    _scrollController.animateTo(
      target.clamp(0.0, _scrollController.position.maxScrollExtent),
      duration: const Duration(milliseconds: 320),
      curve: Curves.easeOutCubic,
    );
  }

  @override
  void dispose() {
    _scrollController.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final primary = _primary;
    if (primary == null) {
      return _EmptyLyrics(onDownloadLyrics: widget.onDownloadLyrics);
    }

    final timed = _timedLines;
    if (timed != null) {
      final active = _activeIndex(timed);
      return ListView.builder(
        controller: _scrollController,
        padding: const EdgeInsets.symmetric(vertical: 48, horizontal: 12),
        itemCount: timed.length,
        itemExtent: 44,
        itemBuilder: (context, index) {
          final line = timed[index];
          final isActive = index == active;
          final isPast = index < active;
          return Align(
            alignment: Alignment.centerLeft,
            child: AnimatedDefaultTextStyle(
              duration: const Duration(milliseconds: 180),
              style: Theme.of(context).textTheme.titleMedium!.copyWith(
                fontSize: isActive ? 20 : 16,
                fontWeight: isActive ? FontWeight.w700 : FontWeight.w500,
                color: isActive
                    ? StrawberryCream.strawberry
                    : StrawberryCream.wine.withValues(
                        alpha: isPast ? 0.45 : 0.72,
                      ),
                height: 1.3,
              ),
              child: Text(
                line.text.isEmpty ? ' ' : line.text,
                maxLines: 2,
                overflow: TextOverflow.ellipsis,
              ),
            ),
          );
        },
      );
    }

    final text = primary.content.text?.trim();
    if (text == null || text.isEmpty) {
      return const _EmptyLyrics();
    }

    return SingleChildScrollView(
      padding: const EdgeInsets.symmetric(vertical: 32, horizontal: 12),
      child: Text(
        text,
        style: Theme.of(context).textTheme.titleMedium?.copyWith(
          height: 1.7,
          color: StrawberryCream.wine.withValues(alpha: 0.85),
        ),
      ),
    );
  }
}

class _EmptyLyrics extends StatelessWidget {
  const _EmptyLyrics({this.onDownloadLyrics});

  final VoidCallback? onDownloadLyrics;

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(
            Icons.lyrics_outlined,
            size: 40,
            color: StrawberryCream.wine.withValues(alpha: 0.3),
          ),
          const SizedBox(height: 12),
          Text(
            '暂无歌词',
            style: Theme.of(
              context,
            ).textTheme.titleMedium?.copyWith(color: StrawberryCream.wineMuted),
          ),
          const SizedBox(height: 4),
          Text(
            '内嵌 LRC 或同名 .lrc 会在这里同步显示',
            style: Theme.of(context).textTheme.bodyMedium,
          ),
          if (onDownloadLyrics != null) ...[
            const SizedBox(height: 16),
            TextButton(onPressed: onDownloadLyrics, child: const Text('下载歌词')),
          ],
        ],
      ),
    );
  }
}
