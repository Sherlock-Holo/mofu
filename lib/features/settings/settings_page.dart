import 'package:flutter/material.dart';
import 'package:go_router/go_router.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/settings/settings_controller.dart';
import 'package:provider/provider.dart';
import 'package:yaru/yaru.dart';

/// App preferences. Playback continues while this page is open.
class SettingsPage extends StatelessWidget {
  const SettingsPage({super.key});

  void _pop(BuildContext context) {
    if (context.canPop()) {
      context.pop();
    } else {
      context.go('/');
    }
  }

  @override
  Widget build(BuildContext context) {
    final settings = context.watch<SettingsController>();

    return Scaffold(
      backgroundColor: Theme.of(context).scaffoldBackgroundColor,
      body: SafeArea(
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(8, 4, 16, 0),
              child: Row(
                children: [
                  YaruIconButton(
                    tooltip: '返回',
                    onPressed: () => _pop(context),
                    icon: const Icon(Icons.arrow_back_rounded),
                  ),
                  Expanded(
                    child: Text(
                      '设置',
                      style: Theme.of(context).textTheme.titleLarge?.copyWith(
                        color: StrawberryCream.wine,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                  ),
                ],
              ),
            ),
            Expanded(
              child: ListView(
                padding: const EdgeInsets.fromLTRB(0, 8, 0, 16),
                children: [
                  SwitchListTile(
                    title: const Text('后台播放时停止唱片动画'),
                    subtitle: const Text('切到后台后暂停播放大页封面旋转，降低耗电'),
                    value: settings.pauseCoverAnimationInBackground,
                    onChanged: settings.setPauseCoverAnimationInBackground,
                  ),
                  if (settings.error != null)
                    Padding(
                      padding: const EdgeInsets.fromLTRB(16, 8, 16, 0),
                      child: Text(
                        settings.error!,
                        style: TextStyle(
                          color: Theme.of(context).colorScheme.error,
                        ),
                      ),
                    ),
                ],
              ),
            ),
          ],
        ),
      ),
    );
  }
}
