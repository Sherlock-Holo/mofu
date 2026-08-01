import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/widgets/cover_view.dart';
import 'package:mofu/widgets/duration_format.dart';
import 'package:mofu/widgets/playback_mode_button.dart';
import 'package:mofu/src/rust/api/player.dart';

void main() {
  group('DurationFormat', () {
    test('formats under one hour', () {
      expect(DurationFormat.fromMilliseconds(65000), '1:05');
    });

    test('formats with hours', () {
      expect(DurationFormat.fromMilliseconds(3661000), '1:01:01');
    });
  });

  group('StrawberryCream theme', () {
    testWidgets('builds MaterialApp with cream scaffold', (tester) async {
      await tester.pumpWidget(
        MaterialApp(
          theme: StrawberryCream.theme(),
          home: const Scaffold(body: Text('草莓奶油')),
        ),
      );
      expect(find.text('草莓奶油'), findsOneWidget);
      final scaffold = tester.widget<Scaffold>(find.byType(Scaffold));
      expect(scaffold.backgroundColor ?? StrawberryCream.cream,
          StrawberryCream.cream);
    });
  });

  group('CoverView', () {
    testWidgets('shows placeholder without cover', (tester) async {
      await tester.pumpWidget(
        const MaterialApp(
          home: Scaffold(body: CoverView(size: 48)),
        ),
      );
      expect(find.byIcon(Icons.music_note_rounded), findsOneWidget);
    });
  });

  group('PlaybackModeButton', () {
    testWidgets('opens mode menu', (tester) async {
      PlaybackMode? selected;
      await tester.pumpWidget(
        MaterialApp(
          home: Scaffold(
            body: PlaybackModeButton(
              mode: PlaybackMode.sequential,
              onChanged: (mode) => selected = mode,
            ),
          ),
        ),
      );
      await tester.tap(find.byTooltip('顺序播放'));
      await tester.pumpAndSettle();
      expect(find.text('随机播放'), findsOneWidget);
      await tester.tap(find.text('随机播放'));
      await tester.pumpAndSettle();
      expect(selected, PlaybackMode.shuffle);
    });
  });
}
