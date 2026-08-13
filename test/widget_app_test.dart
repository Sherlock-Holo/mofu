import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mofu/app/app.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/collection/collection_controller.dart';
import 'package:mofu/features/now_playing/now_playing_page.dart';
import 'package:mofu/features/now_playing/widgets/lyrics_panel.dart';
import 'package:mofu/features/now_playing/widgets/rotating_cover.dart';
import 'package:mofu/features/settings/settings_page.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/settings/settings_controller.dart';
import 'package:mofu/src/rust/api/app.dart';
import 'package:mofu/src/rust/api/collection.dart';
import 'package:mofu/src/rust/api/player.dart';
import 'package:mofu/widgets/playback_bar.dart';
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
    displayTitle: title,
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
    songs: [_song(path: '/music/c.mp3', title: '午后茶')],
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
    kind: PlayerSnapshotKind.full,
    positionMs: positionMs ?? BigInt.from(2500),
    durationMs: BigInt.from(180000),
    state: state,
    volume: 0.8,
    playbackMode: mode,
    currentIndex: BigInt.zero,
    currentFromUpNext: false,
    currentPath: '/music/a.mp3',
    displayTitle: '草莓糖',
    activeLyricsLine: BigInt.one,
    sourcePlaylistId: BigInt.one,
    queue: [
      QueueItem(index: BigInt.zero, path: '/music/a.mp3'),
      QueueItem(index: BigInt.one, path: '/music/b.mp3'),
    ],
    upNext: const [],
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
  SettingsController? settings,
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
      settingsController: settings ?? SettingsController.fake(),
    ),
  );
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 50));
}

Future<void> _pumpNowPlaying(
  WidgetTester tester, {
  required PlayerController player,
  SettingsController? settings,
}) async {
  tester.view.physicalSize = const Size(1200, 800);
  tester.view.devicePixelRatio = 1;
  addTearDown(() {
    tester.view.resetPhysicalSize();
    tester.view.resetDevicePixelRatio();
  });

  await tester.pumpWidget(
    MultiProvider(
      providers: [
        ChangeNotifierProvider<PlayerController>.value(value: player),
        ChangeNotifierProvider<SettingsController>.value(
          value: settings ?? SettingsController.fake(),
        ),
      ],
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

double _coverTurns(WidgetTester tester) {
  final paint = tester.widget<CustomPaint>(
    find.descendant(
      of: find.byType(RotatingCover),
      matching: find.byWidgetPredicate(
        (widget) =>
            widget is CustomPaint && widget.painter is RotatingCoverPainter,
      ),
    ),
  );
  return (paint.painter! as RotatingCoverPainter).turns;
}

/// 等到旋转封面建立后再断言角度。
Future<void> _waitForRotatingCover(WidgetTester tester) async {
  final cover = find.descendant(
    of: find.byType(RotatingCover),
    matching: find.byWidgetPredicate(
      (widget) =>
          widget is CustomPaint && widget.painter is RotatingCoverPainter,
    ),
  );
  for (var i = 0; i < 20 && cover.evaluate().isEmpty; i++) {
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 20));
  }
  expect(cover, findsOneWidget);
}

void main() {
  group('HomePage', () {
    testWidgets('wide layout shows sidebar, song table and playback bar', (
      tester,
    ) async {
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist(), _virtualPlaylist()],
        selectedId: BigInt.one,
      );
      final player = PlayerController.fake(snapshot: _playingSnapshot());

      await _pumpApp(tester, collection: collection, player: player);

      expect(find.text('Mofu'), findsNothing);
      expect(find.text('目录'), findsWidgets);
      expect(find.text('虚拟列表'), findsWidgets);
      expect(find.text('草莓糖'), findsWidgets);
      expect(find.text('奶油云'), findsOneWidget);
      expect(find.byType(PlaybackModeButton), findsOneWidget);
    });

    testWidgets('selecting playlist updates song table', (tester) async {
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist(), _virtualPlaylist()],
        selectedId: BigInt.one,
      );
      final player = PlayerController.fake();

      await _pumpApp(tester, collection: collection, player: player);

      await tester.tap(find.text('最爱'));
      await tester.pump();
      expect(find.text('午后茶'), findsOneWidget);
      expect(find.text('奶油云'), findsNothing);
    });

    testWidgets('empty virtual playlist shows add-files action', (
      tester,
    ) async {
      final emptyVirtual = Playlist(
        id: BigInt.from(2),
        name: '空列表',
        kind: PlaylistKind.virtual,
        songs: const [],
      );
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist(), emptyVirtual],
        selectedId: BigInt.from(2),
      );
      final player = PlayerController.fake(snapshot: _playingSnapshot());

      await _pumpApp(tester, collection: collection, player: player);

      expect(find.text('这个列表还没有歌曲'), findsOneWidget);
      expect(find.text('添加音乐文件'), findsOneWidget);
    });

    testWidgets('play all forwards playlist id to player', (tester) async {
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist(), _virtualPlaylist()],
        selectedId: BigInt.from(2),
      );
      final player = PlayerController.fake();

      await _pumpApp(tester, collection: collection, player: player);
      await tester.tap(find.text('播放全部'));
      await tester.pump();

      expect(player.recordedCommands, contains('playPlaylist:2:null:true'));
    });

    testWidgets('clicking a song forwards its stable path', (tester) async {
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist()],
        selectedId: BigInt.one,
      );
      final player = PlayerController.fake();

      await _pumpApp(tester, collection: collection, player: player);
      await tester.tap(find.text('奶油云'));
      await tester.pump(const Duration(milliseconds: 350));

      expect(
        player.recordedCommands,
        contains('playPlaylist:1:/music/b.mp3:true'),
      );
    });

    testWidgets('right-clicking a song enqueues it via context menu', (
      tester,
    ) async {
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist()],
        selectedId: BigInt.one,
      );
      final player = PlayerController.fake(snapshot: _playingSnapshot());

      await _pumpApp(tester, collection: collection, player: player);

      await tester.tap(find.text('奶油云'), buttons: kSecondaryMouseButton);
      await tester.pumpAndSettle();
      expect(find.text('下一首播放'), findsOneWidget);
      await tester.tap(find.text('下一首播放'));
      await tester.pump();

      expect(player.snapshot?.upNext.single.path, '/music/b.mp3');
      expect(player.recordedCommands, contains('enqueueNext:/music/b.mp3'));
    });

    testWidgets('playback mode button on home bar is wired to player', (
      tester,
    ) async {
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist()],
        selectedId: BigInt.one,
      );
      final player = PlayerController.fake(snapshot: _playingSnapshot());

      await _pumpApp(tester, collection: collection, player: player);

      expect(find.byType(PlaybackModeButton), findsOneWidget);
      expect(find.byTooltip('顺序播放'), findsOneWidget);

      player.setPlaybackMode(PlaybackMode.repeatAll);
      await tester.pump();
      expect(player.playbackMode, PlaybackMode.repeatAll);
      expect(find.byTooltip('列表循环'), findsOneWidget);
    });

    testWidgets('pause forwards to player adapter', (tester) async {
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist()],
        selectedId: BigInt.one,
      );
      final player = PlayerController.fake(snapshot: _playingSnapshot());

      await _pumpApp(tester, collection: collection, player: player);
      player.pause();
      await tester.pump();

      expect(player.recordedCommands, contains('pause'));
      expect(player.isPlaying, isFalse);
    });

    testWidgets('volume updates local snapshot in fake adapter', (
      tester,
    ) async {
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist()],
        selectedId: BigInt.one,
      );
      final player = PlayerController.fake();

      await _pumpApp(tester, collection: collection, player: player);
      player.setVolume(0.5);
      player.setVolume(0.3);
      await tester.pump();

      expect(player.volume, 0.3);
      expect(player.recordedCommands, ['setVolume', 'setVolume']);
    });

    testWidgets('narrow layout uses drawer instead of persistent sidebar', (
      tester,
    ) async {
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
    testWidgets('refreshes title when metadata arrives after path change', (
      tester,
    ) async {
      final player = PlayerController.fake(
        snapshot: PlayerSnapshot(
          kind: PlayerSnapshotKind.full,
          positionMs: BigInt.zero,
          durationMs: BigInt.from(180000),
          state: PlayerState.playing,
          volume: 0.8,
          playbackMode: PlaybackMode.sequential,
          currentIndex: BigInt.zero,
          currentFromUpNext: false,
          currentPath: '/music/dance.flac',
          // 切歌瞬间：队列已更新，标签尚未到达 → 先显示文件名。
          displayTitle: 'dance.flac',
          queue: [QueueItem(index: BigInt.zero, path: '/music/dance.flac')],
          upNext: const [],
        ),
      );
      await _pumpNowPlaying(tester, player: player);

      expect(find.text('dance.flac'), findsOneWidget);
      expect(find.text('未知专辑'), findsOneWidget);
      expect(find.text('未知歌手'), findsOneWidget);

      // path / playing 不变，仅 metadata + displayTitle 到位（复现原 bug 场景）。
      player.emitSnapshot(
        PlayerSnapshot(
          kind: PlayerSnapshotKind.full,
          positionMs: BigInt.zero,
          durationMs: BigInt.from(180000),
          state: PlayerState.playing,
          volume: 0.8,
          playbackMode: PlaybackMode.sequential,
          currentIndex: BigInt.zero,
          currentFromUpNext: false,
          currentPath: '/music/dance.flac',
          displayTitle: 'dance in the forest',
          queue: [QueueItem(index: BigInt.zero, path: '/music/dance.flac')],
          upNext: const [],
          metadata: _meta(
            title: 'dance in the forest',
            artist: 'Falcom Sound Team J.D.K.',
            album: 'ぐるみん オリジナルサウンドトラック',
          ),
        ),
      );
      await tester.pump();

      expect(find.text('dance in the forest'), findsOneWidget);
      expect(find.text('ぐるみん オリジナルサウンドトラック'), findsOneWidget);
      expect(find.text('Falcom Sound Team J.D.K.'), findsOneWidget);
      expect(find.text('dance.flac'), findsNothing);
    });

    testWidgets('highlights active lyric line by position', (tester) async {
      final player = PlayerController.fake(snapshot: _playingSnapshot());
      await _pumpNowPlaying(tester, player: player);
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

    testWidgets('rotating cover animates while playing and stops when paused', (
      tester,
    ) async {
      final player = PlayerController.fake(snapshot: _playingSnapshot());
      await _pumpNowPlaying(tester, player: player);
      await _waitForRotatingCover(tester);

      expect(find.byType(RotatingCover), findsOneWidget);

      final before = _coverTurns(tester);
      await tester.pump(const Duration(seconds: 2));
      expect(_coverTurns(tester), isNot(before));

      await tester.tap(find.byIcon(Icons.pause_rounded));
      await tester.pump();

      final paused = _coverTurns(tester);
      await tester.pump(const Duration(seconds: 2));
      expect(_coverTurns(tester), paused);
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
      await _waitForRotatingCover(tester);

      final first = _coverTurns(tester);
      await tester.pump(const Duration(seconds: 3));
      expect(_coverTurns(tester), isNot(first));

      await tester.pumpWidget(
        const MaterialApp(
          home: Scaffold(
            body: Center(
              child: RotatingCover(cover: null, playing: false, size: 120),
            ),
          ),
        ),
      );
      await _waitForRotatingCover(tester);
      final paused = _coverTurns(tester);
      await tester.pump(const Duration(seconds: 3));
      expect(_coverTurns(tester), paused);
    });

    testWidgets('does not spin after track change while app is inactive', (
      tester,
    ) async {
      await tester.pumpWidget(
        const MaterialApp(
          home: Scaffold(
            body: Center(
              child: RotatingCover(
                key: ValueKey('track-1'),
                cover: null,
                playing: true,
                size: 120,
              ),
            ),
          ),
        ),
      );
      await _waitForRotatingCover(tester);

      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
      await tester.pump();

      await tester.pumpWidget(
        const MaterialApp(
          home: Scaffold(
            body: Center(
              child: RotatingCover(
                key: ValueKey('track-2'),
                cover: null,
                playing: true,
                size: 120,
              ),
            ),
          ),
        ),
      );
      await _waitForRotatingCover(tester);

      final afterTrackChange = _coverTurns(tester);
      await tester.pump(const Duration(seconds: 3));
      expect(_coverTurns(tester), afterTrackChange);
    });

    testWidgets('keeps spinning in background when pauseWhenInactive is false', (
      tester,
    ) async {
      await tester.pumpWidget(
        const MaterialApp(
          home: Scaffold(
            body: Center(
              child: RotatingCover(
                cover: null,
                playing: true,
                pauseWhenInactive: false,
                size: 120,
              ),
            ),
          ),
        ),
      );
      await _waitForRotatingCover(tester);

      final before = _coverTurns(tester);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
      await tester.pump();
      // Frames are disabled while paused; advance wall time then resume to paint.
      await tester.pump(const Duration(seconds: 3));
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pump();
      expect(_coverTurns(tester), isNot(before));
    });

    testWidgets('stops in background when pauseWhenInactive is true', (
      tester,
    ) async {
      await tester.pumpWidget(
        const MaterialApp(
          home: Scaffold(
            body: Center(
              child: RotatingCover(
                cover: null,
                playing: true,
                pauseWhenInactive: true,
                size: 120,
              ),
            ),
          ),
        ),
      );
      await _waitForRotatingCover(tester);

      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
      await tester.pump();
      final paused = _coverTurns(tester);
      await tester.pump(const Duration(seconds: 3));
      // Still paused: timer is cancelled so the angle stays frozen.
      expect(_coverTurns(tester), paused);
    });
  });

  group('Settings', () {
    testWidgets('sidebar opens settings without playback bar or pause', (
      tester,
    ) async {
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist()],
        selectedId: BigInt.one,
      );
      final player = PlayerController.fake(snapshot: _playingSnapshot());
      final settings = SettingsController.fake();

      await _pumpApp(
        tester,
        collection: collection,
        player: player,
        settings: settings,
      );

      expect(find.byTooltip('设置'), findsOneWidget);
      await tester.tap(find.byTooltip('设置'));
      await tester.pumpAndSettle();

      expect(find.byType(SettingsPage), findsOneWidget);
      expect(find.text('后台播放时停止唱片动画'), findsOneWidget);
      expect(find.byType(PlaybackBar), findsNothing);
      expect(player.isPlaying, isTrue);
      expect(player.recordedCommands, isNot(contains('pause')));
    });

    testWidgets('toggle updates controller and records command', (
      tester,
    ) async {
      final collection = CollectionController.fake(
        playlists: [_directoryPlaylist()],
        selectedId: BigInt.one,
      );
      final player = PlayerController.fake(snapshot: _playingSnapshot());
      final settings = SettingsController.fake();

      await _pumpApp(
        tester,
        collection: collection,
        player: player,
        settings: settings,
      );
      await tester.tap(find.byTooltip('设置'));
      await tester.pumpAndSettle();

      expect(settings.pauseCoverAnimationInBackground, isTrue);
      await tester.tap(find.byType(Switch));
      await tester.pumpAndSettle();

      expect(settings.pauseCoverAnimationInBackground, isFalse);
      expect(
        settings.coverAnimationInBackground,
        CoverAnimationInBackground.continue_,
      );
      expect(
        settings.recordedCommands,
        contains(
          'setCoverAnimationInBackground:${CoverAnimationInBackground.continue_}',
        ),
      );
    });

    testWidgets('narrow drawer exposes settings entry', (tester) async {
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

      await tester.tap(find.byIcon(Icons.menu));
      await tester.pumpAndSettle();
      expect(find.byTooltip('设置'), findsOneWidget);

      await tester.tap(find.byTooltip('设置'));
      await tester.pumpAndSettle();
      expect(find.byType(SettingsPage), findsOneWidget);
    });

    testWidgets('rolls back switch when persistence throws', (tester) async {
      final settings = SettingsController()
        ..testSetCoverAnimation = (_) => throw StateError('persist failed');

      await tester.pumpWidget(
        ChangeNotifierProvider<SettingsController>.value(
          value: settings,
          child: MaterialApp(
            theme: StrawberryCream.theme(),
            home: const SettingsPage(),
          ),
        ),
      );

      expect(settings.pauseCoverAnimationInBackground, isTrue);
      await tester.tap(find.byType(Switch));
      await tester.pump();

      expect(settings.pauseCoverAnimationInBackground, isTrue);
      expect(find.textContaining('persist failed'), findsOneWidget);
    });

    testWidgets('now playing respects settings continue in background', (
      tester,
    ) async {
      final player = PlayerController.fake(snapshot: _playingSnapshot());
      final settings = SettingsController.fake(
        coverAnimationInBackground: CoverAnimationInBackground.continue_,
      );
      await _pumpNowPlaying(tester, player: player, settings: settings);
      await _waitForRotatingCover(tester);

      final before = _coverTurns(tester);
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
      await tester.pump();
      await tester.pump(const Duration(seconds: 2));
      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.resumed);
      await tester.pump();
      expect(_coverTurns(tester), isNot(before));
    });

    testWidgets('now playing stops in background with default pause setting', (
      tester,
    ) async {
      final player = PlayerController.fake(snapshot: _playingSnapshot());
      final settings = SettingsController.fake();
      await _pumpNowPlaying(tester, player: player, settings: settings);
      await _waitForRotatingCover(tester);

      tester.binding.handleAppLifecycleStateChanged(AppLifecycleState.paused);
      await tester.pump();
      final paused = _coverTurns(tester);
      await tester.pump(const Duration(seconds: 2));
      expect(_coverTurns(tester), paused);
    });
  });
}
