import 'package:flutter/material.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/library/library_controller.dart';
import 'package:mofu/src/rust/api/library.dart';
import 'package:provider/provider.dart';

/// 左侧播放列表侧栏：目录列表与虚拟列表。
class PlaylistSidebar extends StatelessWidget {
  const PlaylistSidebar({super.key});

  @override
  Widget build(BuildContext context) {
    final library = context.watch<LibraryController>();

    return DecoratedBox(
      decoration: BoxDecoration(
        color: StrawberryCream.creamDeep.withValues(alpha: 0.65),
        border: Border(
          right: BorderSide(
            color: StrawberryCream.strawberry.withValues(alpha: 0.12),
          ),
        ),
      ),
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
                      IconButton(
                        tooltip: '打开音乐目录',
                        onPressed: library.loading
                            ? null
                            : library.pickAndAddDirectory,
                        icon:
                            const Icon(Icons.folder_open_rounded, size: 20),
                      ),
                      IconButton(
                        tooltip: '添加音乐文件',
                        onPressed: library.loading
                            ? null
                            : library.pickAndAddFiles,
                        icon:
                            const Icon(Icons.library_add_rounded, size: 20),
                      ),
                      IconButton(
                        tooltip: '新建虚拟列表',
                        onPressed: library.loading
                            ? null
                            : () => _promptCreateVirtual(context),
                        icon:
                            const Icon(Icons.playlist_add_rounded, size: 20),
                      ),
                      IconButton(
                        tooltip: '刷新当前列表',
                        onPressed: library.loading
                            ? null
                            : library.refreshSelected,
                        icon: const Icon(Icons.refresh_rounded, size: 20),
                      ),
                    ],
                  ),
                ),
                if (library.loading)
                  const Padding(
                    padding: EdgeInsets.only(right: 8),
                    child: SizedBox(
                      width: 16,
                      height: 16,
                      child: CircularProgressIndicator(strokeWidth: 2),
                    ),
                  ),
              ],
            ),
          ),
          const SizedBox(height: 4),
          Expanded(
            child: ListView(
              padding: const EdgeInsets.fromLTRB(8, 0, 8, 16),
              children: [
                const _SectionLabel(label: '目录'),
                if (library.directoryPlaylists.isEmpty)
                  const _EmptyHint(text: '尚未导入目录')
                else
                  for (final playlist in library.directoryPlaylists)
                    _PlaylistTile(playlist: playlist),
                const SizedBox(height: 12),
                const _SectionLabel(label: '虚拟列表'),
                if (library.virtualPlaylists.isEmpty)
                  const _EmptyHint(text: '可创建虚拟列表整理歌曲')
                else
                  for (final playlist in library.virtualPlaylists)
                    _PlaylistTile(playlist: playlist, virtual: true),
              ],
            ),
          ),
        ],
      ),
    );
  }

  Future<void> _promptCreateVirtual(BuildContext context) async {
    final library = context.read<LibraryController>();
    final name = await _promptText(
      context,
      title: '新建虚拟列表',
      hint: '列表名称',
      confirmLabel: '创建',
    );
    if (name == null) {
      return;
    }
    await library.createVirtual(name);
  }
}

class _SectionLabel extends StatelessWidget {
  const _SectionLabel({required this.label});

  final String label;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(8, 8, 8, 4),
      child: Text(
        label,
        style: Theme.of(context).textTheme.labelLarge?.copyWith(
              color: StrawberryCream.wineMuted,
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
      padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 6),
      child: Text(
        text,
        style: Theme.of(context).textTheme.bodySmall?.copyWith(
              color: StrawberryCream.wineMuted,
            ),
      ),
    );
  }
}

class _PlaylistTile extends StatelessWidget {
  const _PlaylistTile({
    required this.playlist,
    this.virtual = false,
  });

  final Playlist playlist;
  final bool virtual;

  @override
  Widget build(BuildContext context) {
    final library = context.watch<LibraryController>();
    final selected = library.selectedId == playlist.id;

    return Material(
      key: ValueKey(playlist.id),
      color: Colors.transparent,
      child: ListTile(
        dense: true,
        selected: selected,
        selectedTileColor: StrawberryCream.strawberry.withValues(alpha: 0.12),
        shape: RoundedRectangleBorder(
          borderRadius: BorderRadius.circular(StrawberryCream.radiusSm),
        ),
        leading: Icon(
          virtual ? Icons.queue_music_rounded : Icons.folder_rounded,
          size: 20,
        ),
        title: Text(
          playlist.name,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
        ),
        subtitle: Text('${playlist.songs.length} 首'),
        onTap: () => library.selectPlaylist(playlist.id),
        onLongPress: virtual
            ? () => _showVirtualActions(context, playlist)
            : () => library.refresh(playlist.id),
        trailing: virtual
            ? IconButton(
                tooltip: '更多',
                icon: const Icon(Icons.more_horiz_rounded, size: 18),
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
    final library = context.read<LibraryController>();
    final action = await showModalBottomSheet<String>(
      context: context,
      backgroundColor: StrawberryCream.cream,
      builder: (context) => SafeArea(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            ListTile(
              leading: const Icon(Icons.drive_file_rename_outline_rounded),
              title: const Text('重命名'),
              onTap: () => Navigator.pop(context, 'rename'),
            ),
            ListTile(
              leading: const Icon(Icons.playlist_add_check_rounded),
              title: const Text('从其他列表导入'),
              onTap: () => Navigator.pop(context, 'import'),
            ),
            ListTile(
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
        await library.renameVirtual(playlist.id, name);
      }
    } else if (action == 'import') {
      await _importFromOtherPlaylist(context, playlist);
    } else if (action == 'delete') {
      final confirmed = await showDialog<bool>(
        context: context,
        builder: (context) => AlertDialog(
          title: const Text('删除虚拟列表'),
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
        await library.deleteVirtual(playlist.id);
      }
    }
  }

  Future<void> _importFromOtherPlaylist(
    BuildContext context,
    Playlist target,
  ) async {
    final library = context.read<LibraryController>();
    final sources = library.playlists
        .where((playlist) => playlist.id != target.id)
        .toList();
    if (sources.isEmpty) {
      ScaffoldMessenger.of(context).showSnackBar(
        const SnackBar(content: Text('没有可导入的其他列表')),
      );
      return;
    }

    final sourceId = await showDialog<BigInt>(
      context: context,
      builder: (context) => SimpleDialog(
        title: const Text('从其他列表导入'),
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
    await library.importFromPlaylist(
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
      title: Text(widget.title),
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
