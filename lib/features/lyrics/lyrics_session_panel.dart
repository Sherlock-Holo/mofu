import 'dart:async';

import 'package:flutter/material.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/src/rust/api/app.dart';
import 'package:mofu/src/rust/api/player.dart';
import 'package:mofu/widgets/duration_format.dart';
import 'package:provider/provider.dart';

/// Non-modal lyrics candidate panel. Stays above [PlaybackBar], never a dialog.
class LyricsSessionPanel extends StatelessWidget {
  const LyricsSessionPanel({super.key, this.maxHeight = 280});

  final double maxHeight;

  @override
  Widget build(BuildContext context) {
    final session = context.select<PlayerController, LyricsSessionSnapshot?>(
      (player) => player.lyricsSession,
    );
    final pending = context.select<PlayerController, LyricsConfirmResult?>(
      (player) => player.pendingLyricsConfirm,
    );
    if (session == null && pending == null) {
      return const SizedBox.shrink();
    }

    return Material(
      color: Theme.of(context).colorScheme.surface,
      elevation: 6,
      child: ConstrainedBox(
        constraints: BoxConstraints(maxHeight: maxHeight),
        child: SafeArea(
          top: false,
          bottom: false,
          child: _PanelBody(session: session, pending: pending),
        ),
      ),
    );
  }
}

class _PanelBody extends StatelessWidget {
  const _PanelBody({required this.session, required this.pending});

  final LyricsSessionSnapshot? session;
  final LyricsConfirmResult? pending;

  @override
  Widget build(BuildContext context) {
    final player = context.watch<PlayerController>();
    final currentPath = player.currentPath;
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 10, 16, 12),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _Header(session: session, currentPath: currentPath),
          if (pending != null) ...[
            const SizedBox(height: 8),
            _ConfirmPrompt(pending: pending!, session: session),
          ],
          if (session != null) ...[
            const SizedBox(height: 8),
            Flexible(child: _SessionStatus(session: session!)),
          ],
          if (session != null &&
              session!.status == LyricsSearchStatus.ready &&
              session!.previewId != null &&
              pending == null) ...[
            const SizedBox(height: 8),
            _Actions(session: session!, currentPath: currentPath),
          ],
        ],
      ),
    );
  }
}

class _Header extends StatelessWidget {
  const _Header({required this.session, required this.currentPath});

  final LyricsSessionSnapshot? session;
  final String? currentPath;

  @override
  Widget build(BuildContext context) {
    final player = context.read<PlayerController>();
    final title = session == null ? '确认保存歌词' : '下载歌词';
    final target = session?.targetPath;
    final isCurrent = target != null && target == currentPath;

    return Row(
      children: [
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(title, style: Theme.of(context).textTheme.titleMedium),
              if (target != null && !isCurrent)
                Padding(
                  padding: const EdgeInsets.only(top: 2),
                  child: Text(
                    '当前播放的不是这首歌，预览前请先「播放并预览」',
                    style: Theme.of(context).textTheme.bodySmall?.copyWith(
                      color: StrawberryCream.wineMuted,
                    ),
                  ),
                ),
            ],
          ),
        ),
        if (session != null)
          TextButton(
            onPressed: () => player.lyricsCancel(sessionId: session!.sessionId),
            child: const Text('全部丢弃'),
          ),
      ],
    );
  }
}

class _SessionStatus extends StatelessWidget {
  const _SessionStatus({required this.session});

  final LyricsSessionSnapshot session;

  @override
  Widget build(BuildContext context) {
    switch (session.status) {
      case LyricsSearchStatus.searching:
        return const Padding(
          padding: EdgeInsets.symmetric(vertical: 16),
          child: Row(
            children: [
              SizedBox(
                width: 18,
                height: 18,
                child: CircularProgressIndicator(strokeWidth: 2),
              ),
              SizedBox(width: 12),
              Text('正在搜索歌词…'),
            ],
          ),
        );
      case LyricsSearchStatus.noResults:
        return const Padding(
          padding: EdgeInsets.symmetric(vertical: 16),
          child: Text('没有找到合适的歌词'),
        );
      case LyricsSearchStatus.failed:
        return Padding(
          padding: const EdgeInsets.symmetric(vertical: 16),
          child: Text(session.errorMessage ?? '搜索歌词失败'),
        );
      case LyricsSearchStatus.ready:
        if (session.candidates.isEmpty) {
          return const Padding(
            padding: EdgeInsets.symmetric(vertical: 16),
            child: Text('没有找到合适的歌词'),
          );
        }
        return _CandidateList(session: session);
    }
  }
}

class _CandidateList extends StatelessWidget {
  const _CandidateList({required this.session});

  final LyricsSessionSnapshot session;

  @override
  Widget build(BuildContext context) {
    final player = context.read<PlayerController>();
    return ListView.builder(
      shrinkWrap: true,
      itemCount: session.candidates.length,
      itemBuilder: (context, index) {
        final candidate = session.candidates[index];
        final selected = session.previewId == candidate.id;
        final duration = candidate.durationMs;
        final details = <String>[
          candidate.providerLabel,
          if (candidate.synced) '同步歌词' else '纯文本',
          if (duration != null) DurationFormat.fromMs(duration),
          if (candidate.instrumental == true) '可能是伴奏/纯音乐',
        ];
        return ListTile(
          dense: true,
          selected: selected,
          title: Text(
            candidate.title,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
          ),
          subtitle: Text(
            [
              if (candidate.artist.trim().isNotEmpty) candidate.artist,
              ...details,
            ].join(' · '),
            maxLines: 2,
            overflow: TextOverflow.ellipsis,
          ),
          trailing: selected
              ? const Icon(
                  Icons.check_rounded,
                  color: StrawberryCream.strawberry,
                )
              : null,
          onTap: () => player.lyricsPreview(
            sessionId: session.sessionId,
            candidateId: candidate.id,
          ),
        );
      },
    );
  }
}

class _Actions extends StatelessWidget {
  const _Actions({required this.session, required this.currentPath});

  final LyricsSessionSnapshot session;
  final String? currentPath;

  @override
  Widget build(BuildContext context) {
    final player = context.read<PlayerController>();
    final previewId = session.previewId;
    final isCurrent = currentPath == session.targetPath;

    return Row(
      children: [
        if (!isCurrent)
          OutlinedButton(
            onPressed: () => player.lyricsPlayAndPreview(
              sessionId: session.sessionId,
              path: session.targetPath,
            ),
            child: const Text('播放并预览'),
          ),
        const Spacer(),
        FilledButton(
          onPressed: previewId == null
              ? null
              : () {
                  unawaited(
                    player.lyricsConfirm(
                      sessionId: session.sessionId,
                      candidateId: previewId,
                      action: LyricsConfirmAction.save,
                    ),
                  );
                },
          child: const Text('保存'),
        ),
      ],
    );
  }
}

class _ConfirmPrompt extends StatelessWidget {
  const _ConfirmPrompt({required this.pending, required this.session});

  final LyricsConfirmResult pending;
  final LyricsSessionSnapshot? session;

  @override
  Widget build(BuildContext context) {
    final player = context.read<PlayerController>();
    final previewId = session?.previewId;
    final canConfirm = session != null && previewId != null;

    late final String message;
    late final String actionLabel;
    late final LyricsConfirmAction action;
    switch (pending.kind) {
      case LyricsConfirmResultKind.needsReplace:
        message = '已有同名歌词文件，确认替换？';
        actionLabel = '确认替换';
        action = LyricsConfirmAction.replaceExistingSidecar;
      case LyricsConfirmResultKind.needsStemCollisionAck:
        message = '存在同名音频冲突，将保存到应用数据目录';
        actionLabel = '保存到数据目录';
        action = LyricsConfirmAction.preferDataDir;
      case LyricsConfirmResultKind.saved:
        return const SizedBox.shrink();
    }

    return DecoratedBox(
      decoration: BoxDecoration(
        color: StrawberryCream.strawberry.withValues(alpha: 0.08),
        borderRadius: BorderRadius.circular(StrawberryCream.radiusSm),
      ),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(12, 8, 12, 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text(message, style: Theme.of(context).textTheme.bodyMedium),
            const SizedBox(height: 8),
            Row(
              children: [
                TextButton(
                  onPressed: session == null
                      ? null
                      : () =>
                            player.lyricsCancel(sessionId: session!.sessionId),
                  child: const Text('取消'),
                ),
                const Spacer(),
                FilledButton(
                  onPressed: canConfirm
                      ? () {
                          unawaited(
                            player.lyricsConfirm(
                              sessionId: session!.sessionId,
                              candidateId: previewId,
                              action: action,
                            ),
                          );
                        }
                      : null,
                  child: Text(actionLabel),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
