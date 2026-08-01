import 'package:flutter/material.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/src/rust/api/player.dart';

/// 四种播放模式循环切换按钮。
class PlaybackModeButton extends StatelessWidget {
  const PlaybackModeButton({
    super.key,
    required this.mode,
    required this.onChanged,
  });

  final PlaybackMode mode;
  final ValueChanged<PlaybackMode> onChanged;

  static const _order = [
    PlaybackMode.sequential,
    PlaybackMode.shuffle,
    PlaybackMode.repeatOne,
    PlaybackMode.repeatAll,
  ];

  static IconData iconFor(PlaybackMode mode) => switch (mode) {
        PlaybackMode.sequential => Icons.arrow_right_alt_rounded,
        PlaybackMode.shuffle => Icons.shuffle_rounded,
        PlaybackMode.repeatOne => Icons.repeat_one_rounded,
        PlaybackMode.repeatAll => Icons.repeat_rounded,
      };

  static String labelFor(PlaybackMode mode) => switch (mode) {
        PlaybackMode.sequential => '顺序播放',
        PlaybackMode.shuffle => '随机播放',
        PlaybackMode.repeatOne => '单曲循环',
        PlaybackMode.repeatAll => '列表循环',
      };

  @override
  Widget build(BuildContext context) {
    return IconButton(
      tooltip: labelFor(mode),
      onPressed: () => _openMenu(context),
      icon: Icon(iconFor(mode), color: StrawberryCream.wine),
    );
  }

  Future<void> _openMenu(BuildContext context) async {
    final button = context.findRenderObject()! as RenderBox;
    final overlay =
        Navigator.of(context).overlay!.context.findRenderObject()! as RenderBox;
    final topLeft = button.localToGlobal(Offset.zero, ancestor: overlay);
    final bottomRight = button.localToGlobal(
      button.size.bottomRight(Offset.zero),
      ancestor: overlay,
    );
    final position = RelativeRect.fromRect(
      Rect.fromPoints(topLeft, bottomRight),
      Offset.zero & overlay.size,
    );

    final selected = await showMenu<PlaybackMode>(
      context: context,
      position: position,
      initialValue: mode,
      items: [
        for (final item in _order)
          PopupMenuItem(
            value: item,
            child: Row(
              children: [
                Icon(
                  iconFor(item),
                  size: 18,
                  color: item == mode
                      ? StrawberryCream.strawberry
                      : StrawberryCream.wineMuted,
                ),
                const SizedBox(width: 10),
                Text(labelFor(item)),
              ],
            ),
          ),
      ],
    );
    if (selected != null) {
      onChanged(selected);
    }
  }
}
