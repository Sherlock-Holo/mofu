import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mofu/app/app.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/features/now_playing/now_playing_page.dart';
import 'package:mofu/features/now_playing/widgets/lyrics_panel.dart';
import 'package:mofu/features/now_playing/widgets/rotating_cover.dart';
import 'package:mofu/collection/collection_controller.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/src/rust/api/collection.dart';
import 'package:mofu/src/rust/api/player.dart';
import 'package:mofu/widgets/playback_mode_button.dart';
import 'package:provider/provider.dart';

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

Song _song({
  required String path,
  required String title,
  String artist = '歌手',
  String album = '专辑',
  int durationMs = 180000,
}) {
  return Song(
    path: path,
    durationMs: BigInt.from(durationMs),
    metadata: _meta(title: title, artist: artist, album: album),
  );
}

Playlist _directoryPlaylist() {
  return Playlist(
    id: BigInt.one,
    name: '本地音乐',
    kind: PlaylistKind.directory,
    directoryPath: '/music',
    songs: [
      _song(path: '/music/a.mp3', title: '草莓糖'),
      _song(path: '/music/b.mp3', title: '奶油云'),
    ],
  );
}

Playlist _virtualPlaylist() {
  return Playlist(
    id: BigInt.from(2),
    name: '最爱',
    kind: PlaylistKind.virtual,
    songs: [
      _song(path: '/music/c.mp3', title: '午后茶'),
    ],
  );
}

List<Lyrics> _timedLyrics() {
  return [
    Lyrics(
      source: LyricsSource.embedded,
      content: LyricsContent(
        kind: LyricsContentKind.timed,
        lines: [
          LyricsLine(
            startMs: BigInt.from(0),
            endMs: BigInt.from(2000),
            text: '第一行歌词',
          ),
          LyricsLine(
            startMs: BigInt.from(2000),
            endMs: BigInt.from(4000),
            text: '第二行歌词',
          ),
          LyricsLine(
            startMs: BigInt.from(4000),
            endMs: BigInt.from(6000),
            text: '第三行歌词',
          ),
        ],
      ),
    ),
  ];
}

PlayerSnapshot _playingSnapshot({
  BigInt? positionMs,
  PlaybackMode mode = PlaybackMode.sequential,
  PlayerState state = PlayerState.playing,
}) {
  return PlayerSnapshot(
    positionMs: positionMs ?? BigInt.from(2500),
    durationMs: BigInt.from(180000),
    state: state,
    volume: 0.8,
    playbackMode: mode,
    currentIndex: BigInt.zero,
    queue: [
      QueueItem(index: BigInt.zero, path: '/music/a.mp3'),
      QueueItem(index: BigInt.one, path: '/music/b.mp3'),
    ],
    metadata: _meta(
      title: '草莓糖',
      artist: '奶油乐队',
      album: '甜点',
      lyrics: _timedLyrics(),
    ),
  );
}

Future<void> _pumpApp(
  WidgetTester tester, {
  required CollectionController collection,
  required PlayerController player,
  Size size = const Size(1200, 800),
}) async {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(() {
    tester.view.resetPhysicalSize();
    tester.view.resetDevicePixelRatio();
  });

  await tester.pumpWidget(
    MofuApp(
      collectionController: collection,
      playerController: player,
    ),
  );
  // 播放中封面旋转是无限动画，不能 pumpAndSettle。
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 50));
}

Future<void> _pumpNowPlaying(
  WidgetTester tester, {
  required PlayerController player,
}) async {
  tester.view.physicalSize = const Size(1200, 800);
  tester.view.devicePixelRatio = 1;
  addTearDown(() {
    tester.view.resetPhysicalSize();
    tester.view.resetDevicePixelRatio();
  });

  await tester.pumpWidget(
    ChangeNotifierProvider<PlayerController>.value(
      value: player,
      child: MaterialApp(
        theme: StrawberryCream.theme(),
        home: const NowPlayingPage(),
      ),
    ),
  );
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 50));
}

TextStyle _styleAround(WidgetTester tester, Finder finder) {
  return DefaultTextStyle.of(tester.element(finder)).style;
}

Matrix4 _coverMatrix(WidgetTester tester) {
  final transform = tester.widget<Transform>(
    find.descendant(
      of: find.byType(RotatingCover),
      matching: find.byType(Transform),
    ),
  );
  return Matrix4.copy(transform.transform);
}

void main() {
  group('HomePage', () {
    testWidgets('wide layout shows sidebar, song table and playback bar',
        (tester) async {
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist(), _virtualPlaylist()],
        selectedId: BigInt.one,
      );
      final player = PlayerController.fake(snapshot: _playingSnapshot());

      await _pumpApp(tester, collection: collection, player: player);

      expect(find.text('Mofu'), findsNothing);
      expect(find.text('目录'), findsWidgets);
      expect(find.text('虚拟列表'), findsWidgets);
      expect(find.text('本地音乐'), findsWidgets);
      expect(find.text('最爱'), findsOneWidget);
      expect(find.text('草莓糖'), findsWidgets);
      expect(find.text('奶油云'), findsOneWidget);
      expect(find.text('歌曲'), findsOneWidget);
      expect(find.byType(PlaybackModeButton), findsOneWidget);
      expect(find.byTooltip('下一首'), findsOneWidget);
    });

    testWidgets('selecting playlist updates song table', (tester) async {
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist(), _virtualPlaylist()],
        selectedId: BigInt.one,
      );
      final player = PlayerController.fake();

      await _pumpApp(tester, collection: collection, player: player);

      expect(find.text('草莓糖'), findsOneWidget);
      expect(find.text('午后茶'), findsNothing);

      await tester.tap(find.text('最爱'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 50));

      expect(collection.selectedId, BigInt.from(2));
      expect(find.text('午后茶'), findsOneWidget);
      expect(find.text('奶油云'), findsNothing);
    });

    testWidgets('header follows browsed playlist, not playing track',
        (tester) async {
      final emptyVirtual = Playlist(
        id: BigInt.from(3),
        name: '空列表',
        kind: PlaylistKind.virtual,
        songs: const [],
      );
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist(), emptyVirtual],
        selectedId: BigInt.one,
      );
      final player = PlayerController.fake(snapshot: _playingSnapshot());

      await _pumpApp(tester, collection: collection, player: player);

      // 正在播「草莓糖」，但头部应是浏览中的「本地音乐」。
      expect(find.text('本地音乐'), findsWidgets);
      expect(find.text('2 首'), findsWidgets);

      await tester.tap(find.text('空列表'));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 50));

      expect(find.text('空列表'), findsWidgets);
      expect(find.text('0 首'), findsWidgets);
      // 播放未停，底栏仍显示当前曲。
      expect(find.text('草莓糖'), findsWidgets);
      expect(player.currentIndex, BigInt.zero);
    });

    testWidgets('play all loads browsed playlist into queue from start',
        (tester) async {
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist(), _virtualPlaylist()],
        selectedId: BigInt.from(2),
      );
      final player = PlayerController.fake();

      await _pumpApp(tester, collection: collection, player: player);

      expect(find.text('播放全部'), findsOneWidget);
      await tester.tap(find.text('播放全部'));
      await tester.pump();

      expect(player.queue.map((item) => item.path), ['/music/c.mp3']);
      expect(player.currentIndex, BigInt.zero);
      expect(player.isPlaying, isTrue);
      expect(player.sourcePlaylistId, BigInt.from(2));
    });

    testWidgets('playback mode button on home bar is wired to player',
        (tester) async {
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist()],
        selectedId: BigInt.one,
      );
      final player = PlayerController.fake(snapshot: _playingSnapshot());

      await _pumpApp(tester, collection: collection, player: player);

      expect(find.byType(PlaybackModeButton), findsOneWidget);
      expect(find.byTooltip('顺序播放'), findsOneWidget);

      // 菜单交互由 widget_foundation_test 覆盖；这里验证底栏接线与图标刷新。
      player.setPlaybackMode(PlaybackMode.repeatAll);
      await tester.pump();
      expect(player.playbackMode, PlaybackMode.repeatAll);
      expect(find.byTooltip('列表循环'), findsOneWidget);
    });

    testWidgets('pause persists playback session with playlist and path',
        (tester) async {
      PlaybackSession? saved;
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist()],
        selectedId: BigInt.one,
      );
      final player = PlayerController.fake(snapshot: _playingSnapshot());
      player.sourcePlaylistId = BigInt.one;

      await _pumpApp(tester, collection: collection, player: player);
      // initState 会先接到 CollectionController；测试覆盖后再验证触发。
      player.persistPlaybackSession = (session) async {
        saved = session;
      };
      player.pause();
      await tester.pump();

      expect(saved?.playlistId, BigInt.one);
      expect(saved?.path, '/music/a.mp3');
      expect(saved?.positionMs, BigInt.from(2500));
    });

    testWidgets('volume persist uses trailing 400ms debounce', (tester) async {
      final volumes = <double>[];
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist()],
        selectedId: BigInt.one,
      );
      final player = PlayerController.fake();

      await _pumpApp(tester, collection: collection, player: player);
      player.persistVolume = (volume) async {
        volumes.add(volume);
      };
      player.setVolume(0.5);
      player.setVolume(0.3);
      await tester.pump(const Duration(milliseconds: 399));
      expect(volumes, isEmpty);

      await tester.pump(const Duration(milliseconds: 1));
      expect(volumes, [0.3]);

      player.setVolume(0.1);
      player.flushVolumePersist();
      expect(volumes, [0.3, 0.1]);
    });

    testWidgets('narrow layout uses drawer instead of persistent sidebar',
        (tester) async {
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist()],
        selectedId: BigInt.one,
      );
      final player = PlayerController.fake();

      await _pumpApp(
        tester,
        collection: collection,
        player: player,
        size: const Size(700, 900),
      );

      expect(find.text('虚拟列表'), findsNothing);
      await tester.tap(find.byIcon(Icons.menu));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.text('虚拟列表'), findsOneWidget);
      expect(find.text('本地音乐'), findsWidgets);
    });
  });

  group('NowPlayingPage', () {
    testWidgets('highlights active lyric line by position', (tester) async {
      final player = PlayerController.fake(snapshot: _playingSnapshot());
      await _pumpNowPlaying(tester, player: player);
      // 等歌词行 AnimatedDefaultTextStyle 过渡完成。
      await tester.pump(const Duration(milliseconds: 200));

      expect(find.byType(LyricsPanel), findsOneWidget);
      expect(find.text('第二行歌词'), findsOneWidget);
      expect(
        _styleAround(tester, find.text('第二行歌词')).color,
        StrawberryCream.strawberry,
      );
      expect(
        _styleAround(tester, find.text('第一行歌词')).color,
        isNot(StrawberryCream.strawberry),
      );
    });

    testWidgets('rotating cover animates while playing and stops when paused',
        (tester) async {
      final player = PlayerController.fake(snapshot: _playingSnapshot());
      await _pumpNowPlaying(tester, player: player);

      expect(find.byType(RotatingCover), findsOneWidget);

      final before = _coverMatrix(tester);
      await tester.pump(const Duration(seconds: 2));
      expect(_coverMatrix(tester).storage, isNot(before.storage));

      await tester.tap(find.byIcon(Icons.pause_rounded));
      await tester.pump();

      final paused = _coverMatrix(tester);
      await tester.pump(const Duration(seconds: 2));
      expect(_coverMatrix(tester).storage, paused.storage);
      expect(player.isPlaying, isFalse);
    });
  });

  group('LyricsPanel', () {
    testWidgets('marks timed line at position as active', (tester) async {
      await tester.pumpWidget(
        MaterialApp(
          theme: StrawberryCream.theme(),
          home: Scaffold(
            body: LyricsPanel(
              lyrics: _timedLyrics(),
              positionMs: BigInt.from(2500),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();

      final active = _styleAround(tester, find.text('第二行歌词'));
      expect(active.color, StrawberryCream.strawberry);
      expect(active.fontWeight, FontWeight.w700);
    });
  });

  group('RotatingCover', () {
    testWidgets('starts and stops rotation with playing flag', (tester) async {
      await tester.pumpWidget(
        const MaterialApp(
          home: Scaffold(
            body: Center(
              child: RotatingCover(cover: null, playing: true, size: 120),
            ),
          ),
        ),
      );

      final first = _coverMatrix(tester);
      await tester.pump(const Duration(seconds: 3));
      expect(_coverMatrix(tester).storage, isNot(first.storage));

      await tester.pumpWidget(
        const MaterialApp(
          home: Scaffold(
            body: Center(
              child: RotatingCover(cover: null, playing: false, size: 120),
            ),
          ),
        ),
      );
      await tester.pump();
      final paused = _coverMatrix(tester);
      await tester.pump(const Duration(seconds: 3));
      expect(_coverMatrix(tester).storage, paused.storage);
    });
  });
}
