import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:integration_test/integration_test.dart';
import 'package:mofu/app/app.dart';
import 'package:mofu/features/now_playing/widgets/lyrics_panel.dart';
import 'package:mofu/library/library_controller.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/src/rust/api/library.dart';
import 'package:mofu/src/rust/api/player.dart';
import 'package:mofu/widgets/playback_mode_button.dart';

TrackMetadata _meta({
  String? title,
  String? artist,
  String? album,
  List<Lyrics> lyrics = const [],
}) {
  return TrackMetadata(
    title: title,
    artist: artist,
    album: album,
    lyrics: lyrics,
  );
}

void main() {
  IntegrationTestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('home and now-playing work with injectable fake controllers',
      (tester) async {
    final songs = [
      Song(
        path: '/music/a.mp3',
        durationMs: BigInt.from(180000),
        metadata: _meta(title: '草莓糖', artist: '奶油乐队', album: '甜点'),
      ),
      Song(
        path: '/music/b.mp3',
        durationMs: BigInt.from(200000),
        metadata: _meta(title: '奶油云', artist: '奶油乐队', album: '甜点'),
      ),
    ];
    final playlists = [
      Playlist(
        id: BigInt.one,
        name: '本地音乐',
        kind: PlaylistKind.directory,
        directoryPath: '/music',
        songs: songs,
      ),
      Playlist(
        id: BigInt.from(2),
        name: '最爱',
        kind: PlaylistKind.virtual,
        songs: [songs.first],
      ),
    ];

    final library = LibraryController.fake(
      playlists: playlists,
      selectedId: BigInt.one,
    );
    final player = PlayerController.fake(
      snapshot: PlayerSnapshot(
        positionMs: BigInt.from(2500),
        durationMs: BigInt.from(180000),
        state: PlayerState.playing,
        volume: 0.7,
        playbackMode: PlaybackMode.sequential,
        currentIndex: BigInt.zero,
        queue: [
          QueueItem(index: BigInt.zero, path: '/music/a.mp3'),
          QueueItem(index: BigInt.one, path: '/music/b.mp3'),
        ],
        metadata: _meta(
          title: '草莓糖',
          artist: '奶油乐队',
          album: '甜点',
          lyrics: [
            Lyrics(
              source: LyricsSource.embedded,
              content: LyricsContent(
                kind: LyricsContentKind.timed,
                lines: [
                  LyricsLine(
                    startMs: BigInt.zero,
                    endMs: BigInt.from(2000),
                    text: '第一行歌词',
                  ),
                  LyricsLine(
                    startMs: BigInt.from(2000),
                    endMs: BigInt.from(4000),
                    text: '第二行歌词',
                  ),
                ],
              ),
            ),
          ],
        ),
      ),
    );

    tester.view.physicalSize = const Size(1280, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(() {
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
    });

    await tester.pumpWidget(
      MofuApp(
        libraryController: library,
        playerController: player,
      ),
    );
    // 播放中封面旋转是无限动画，不能 pumpAndSettle。
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    expect(find.text('本地音乐'), findsWidgets);
    expect(find.text('奶油云'), findsOneWidget);
    expect(find.byType(PlaybackModeButton), findsOneWidget);

    await tester.tap(find.text('最爱'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    expect(library.selectedId, BigInt.from(2));
    expect(find.text('草莓糖'), findsWidgets);

    await tester.tap(find.byIcon(Icons.chevron_right_rounded));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));

    expect(find.text('正在播放'), findsOneWidget);
    expect(find.byType(LyricsPanel), findsOneWidget);
    expect(find.text('第二行歌词'), findsOneWidget);

    await tester.tap(find.byTooltip('返回首页'));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 50));
    expect(find.text('目录'), findsOneWidget);

    // 模式菜单在独立组件测试中覆盖；这里验证控制器可驱动底栏模式。
    player.setPlaybackMode(PlaybackMode.shuffle);
    await tester.pump();
    expect(find.byTooltip('随机播放'), findsWidgets);
    expect(player.playbackMode, PlaybackMode.shuffle);
  });
}
