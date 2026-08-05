import 'package:flutter/material.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/src/rust/api/player.dart';
import 'package:yaru/yaru.dart';

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
    // 底栏需要图标按钮形态；YaruPopupMenuButton 偏下拉描边按钮，不适配此处。
    return YaruPopupMenuButton<PlaybackMode>(
      tooltip: labelFor(mode),
      initialValue: mode,
      showArrow: false,
      padding: EdgeInsets.zero,
      childPadding: const EdgeInsets.all(8),
      style: ButtonStyle(
        backgroundColor: WidgetStateProperty.all(Colors.transparent),
        side: WidgetStateProperty.all(BorderSide.none),
        shape: WidgetStateProperty.all(const CircleBorder()),
        minimumSize: WidgetStateProperty.all(const Size(40, 40)),
        tapTargetSize: MaterialTapTargetSize.shrinkWrap,
      ),
      onSelected: onChanged,
      itemBuilder: (context) => [
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
      child: Icon(iconFor(mode), color: StrawberryCream.wine),
    );
  }
}
