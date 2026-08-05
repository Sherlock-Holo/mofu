import 'package:flutter/material.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/collection/collection_controller.dart';
import 'package:mofu/src/rust/api/collection.dart';
import 'package:provider/provider.dart';
import 'package:yaru/yaru.dart';

/// 左侧播放列表侧栏：目录列表与虚拟列表。
class PlaylistSidebar extends StatelessWidget {
  const PlaylistSidebar({super.key});

  @override
  Widget build(BuildContext context) {
    final collection = context.watch<CollectionController>();

    return ColoredBox(
      color: Theme.of(context).colorScheme.surface,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(8, 12, 8, 0),
            child: Row(
              children: [
                Expanded(
                  child: Wrap(
                    spacing: 4,
                    children: [
                      YaruIconButton(
                        tooltip: '打开音乐目录',
                        onPressed: collection.loading
                            ? null
                            : collection.pickAndAddDirectory,
                        icon: const Icon(Icons.folder_open_rounded),
                      ),
                      YaruIconButton(
                        tooltip: '添加音乐文件',
                        onPressed: collection.loading
                            ? null
                            : collection.pickAndAddFiles,
                        icon: const Icon(Icons.library_add_rounded),
                      ),
                      YaruIconButton(
                        tooltip: '新建虚拟列表',
                        onPressed: collection.loading
                            ? null
                            : () => _promptCreateVirtual(context),
                        icon: const Icon(Icons.playlist_add_rounded),
                      ),
                      YaruIconButton(
                        tooltip: '刷新当前列表',
                        onPressed: collection.loading
                            ? null
                            : collection.refreshSelected,
                        icon: const Icon(Icons.refresh_rounded),
                      ),
                    ],
                  ),
                ),
                if (collection.loading)
                  const Padding(
                    padding: EdgeInsets.only(right: 8),
                    child: SizedBox.square(
                      dimension: 18,
                      child: FittedBox(
                        child: YaruCircularProgressIndicator(strokeWidth: 2),
                      ),
                    ),
                  ),
              ],
            ),
          ),
          const SizedBox(height: 4),
          Expanded(
            child: ListView(
              padding: const EdgeInsets.fromLTRB(0, 0, 0, 16),
              children: [
                const _SectionLabel(label: '目录'),
                if (collection.directoryPlaylists.isEmpty)
                  const _EmptyHint(text: '尚未导入目录')
                else
                  for (final playlist in collection.directoryPlaylists)
                    _PlaylistTile(playlist: playlist),
                const SizedBox(height: 12),
                const _SectionLabel(label: '虚拟列表'),
                if (collection.virtualPlaylists.isEmpty)
                  const _EmptyHint(text: '可创建虚拟列表整理歌曲')
                else
                  for (final playlist in collection.virtualPlaylists)
                    _PlaylistTile(playlist: playlist, virtual: true),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Future<void> _promptCreateVirtual(BuildContext context) async {
    final collection = context.read<CollectionController>();
    final name = await _promptText(
      context,
      title: '新建虚拟列表',
      hint: '列表名称',
      confirmLabel: '创建',
    );
    if (name == null) {
      return;
    }
    await collection.createVirtual(name);
  }
}

class _SectionLabel extends StatelessWidget {
  const _SectionLabel({required this.label});

  final String label;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 8, 16, 4),
      child: Text(
        label,
        style: Theme.of(context).textTheme.labelLarge?.copyWith(
          color: StrawberryCream.wineMuted,
          fontWeight: FontWeight.w600,
          letterSpacing: 0.4,
        ),
      ),
    );
  }
}

class _EmptyHint extends StatelessWidget {
  const _EmptyHint({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(16, 6, 16, 6),
      child: Text(
        text,
        style: Theme.of(
          context,
        ).textTheme.bodySmall?.copyWith(color: StrawberryCream.wineMuted),
      ),
    );
  }
}

class _PlaylistTile extends StatelessWidget {
  const _PlaylistTile({required this.playlist, this.virtual = false});

  final Playlist playlist;
  final bool virtual;

  @override
  Widget build(BuildContext context) {
    final collection = context.watch<CollectionController>();
    final selected = collection.selectedId == playlist.id;

    return GestureDetector(
      key: ValueKey(playlist.id),
      onLongPress: virtual
          ? () => _showVirtualActions(context, playlist)
          : () => collection.refresh(playlist.id),
      child: YaruMasterTile(
        selected: selected,
        leading: Icon(
          virtual ? Icons.queue_music_rounded : Icons.folder_rounded,
          size: 20,
        ),
        title: Text(playlist.name),
        subtitle: Text('${playlist.songs.length} 首'),
        onTap: () => collection.selectPlaylist(playlist.id),
        trailing: virtual
            ? YaruIconButton(
                tooltip: '更多',
                icon: const Icon(Icons.more_horiz_rounded),
                onPressed: () => _showVirtualActions(context, playlist),
              )
            : null,
      ),
    );
  }

  Future<void> _showVirtualActions(
    BuildContext context,
    Playlist playlist,
  ) async {
    final collection = context.read<CollectionController>();
    final action = await showModalBottomSheet<String>(
      context: context,
      backgroundColor: StrawberryCream.cream,
      builder: (context) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            YaruMasterTile(
              leading: const Icon(Icons.drive_file_rename_outline_rounded),
              title: const Text('重命名'),
              onTap: () => Navigator.pop(context, 'rename'),
            ),
            YaruMasterTile(
              leading: const Icon(Icons.playlist_add_check_rounded),
              title: const Text('从其他列表导入'),
              onTap: () => Navigator.pop(context, 'import'),
            ),
            YaruMasterTile(
              leading: const Icon(Icons.delete_outline_rounded),
              title: const Text('删除'),
              onTap: () => Navigator.pop(context, 'delete'),
            ),
          ],
        ),
      ),
    );

    if (!context.mounted) {
      return;
    }
    if (action == 'rename') {
      final name = await _promptText(
        context,
        title: '重命名列表',
        hint: '新名称',
        initial: playlist.name,
        confirmLabel: '保存',
      );
      if (name != null && context.mounted) {
        await collection.renameVirtual(playlist.id, name);
      }
    } else if (action == 'import') {
      await _importFromOtherPlaylist(context, playlist);
    } else if (action == 'delete') {
      final confirmed = await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          titlePadding: EdgeInsets.zero,
          title: const YaruDialogTitleBar(
            title: Text('删除虚拟列表'),
            isDraggable: false,
            onDrag: null,
            onShowMenu: null,
          ),
          content: Text('确定删除「${playlist.name}」？'),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(context, false),
              child: const Text('取消'),
            ),
            FilledButton(
              onPressed: () => Navigator.pop(context, true),
              child: const Text('删除'),
            ),
          ],
        ),
      );
      if (confirmed == true) {
        await collection.deleteVirtual(playlist.id);
      }
    }
  }

  Future<void> _importFromOtherPlaylist(
    BuildContext context,
    Playlist target,
  ) async {
    final collection = context.read<CollectionController>();
    final sources = collection.playlists
        .where((playlist) => playlist.id != target.id)
        .toList();
    if (sources.isEmpty) {
      ScaffoldMessenger.of(
        context,
      ).showSnackBar(const SnackBar(content: Text('没有可导入的其他列表')));
      return;
    }

    final sourceId = await showDialog<BigInt>(
      context: context,
      builder: (context) => SimpleDialog(
        titlePadding: EdgeInsets.zero,
        title: const YaruDialogTitleBar(
          title: Text('从其他列表导入'),
          isDraggable: false,
          onDrag: null,
          onShowMenu: null,
        ),
        children: [
          for (final source in sources)
            SimpleDialogOption(
              onPressed: () => Navigator.pop(context, source.id),
              child: Text('${source.name}（${source.songs.length} 首）'),
            ),
        ],
      ),
    );
    if (sourceId == null || !context.mounted) {
      return;
    }
    await collection.importFromPlaylist(
      targetId: target.id,
      sourceId: sourceId,
    );
  }
}

Future<String?> _promptText(
  BuildContext context, {
  required String title,
  required String hint,
  required String confirmLabel,
  String? initial,
}) {
  return showDialog<String>(
    context: context,
    builder: (context) => _PromptTextDialog(
      title: title,
      hint: hint,
      confirmLabel: confirmLabel,
      initial: initial,
    ),
  );
}

/// Owns [TextEditingController] for the dialog lifetime so it is not disposed
/// while the route (and TextField) is still tearing down.
class _PromptTextDialog extends StatefulWidget {
  const _PromptTextDialog({
    required this.title,
    required this.hint,
    required this.confirmLabel,
    this.initial,
  });

  final String title;
  final String hint;
  final String confirmLabel;
  final String? initial;

  @override
  State<_PromptTextDialog> createState() => _PromptTextDialogState();
}

class _PromptTextDialogState extends State<_PromptTextDialog> {
  late final TextEditingController _controller;

  @override
  void initState() {
    super.initState();
    _controller = TextEditingController(text: widget.initial ?? '');
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      titlePadding: EdgeInsets.zero,
      title: YaruDialogTitleBar(
        title: Text(widget.title),
        isDraggable: false,
        onDrag: null,
        onShowMenu: null,
      ),
      content: TextField(
        controller: _controller,
        autofocus: true,
        decoration: InputDecoration(hintText: widget.hint),
        onSubmitted: (value) => Navigator.pop(context, value),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('取消'),
        ),
        FilledButton(
          onPressed: () => Navigator.pop(context, _controller.text),
          child: Text(widget.confirmLabel),
        ),
      ],
    );
  }
}
