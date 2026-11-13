import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:mofu/app/app.dart';
import 'package:mofu/app/theme.dart';
import 'package:mofu/collection/collection_controller.dart';
import 'package:mofu/features/lyrics/lyrics_session_panel.dart';
import 'package:mofu/features/now_playing/now_playing_page.dart';
import 'package:mofu/player/player_controller.dart';
import 'package:mofu/settings/settings_controller.dart';
import 'package:mofu/src/rust/api/collection.dart';
import 'package:mofu/src/rust/api/player.dart';
import 'package:mofu/widgets/playback_bar.dart';
import 'package:provider/provider.dart';

/// Widget-rendered preview screenshots for layout review.
///
/// These are not device screenshots. Fake test data is labeled in-frame.
/// Output lives under /workspace/artifacts (outside the repo) so this file
/// does not add production assets or new dependencies.
const _previewDir = '/workspace/artifacts/mofu-lyrics-ui-preview';
const _bannerText = '测试数据 · 预览布局，非实机截图';

TrackMetadata _meta({List<Lyrics> lyrics = const []}) {
  return TrackMetadata(
    title: '草莓糖',
    artist: '奶油乐队',
    album: '甜点',
    lyrics: lyrics,
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
  ];
}

Song _song({required String path, required String title}) {
  return Song(
    path: path,
    durationMs: BigInt.from(180000),
    displayTitle: title,
    metadata: TrackMetadata(title: title, artist: '奶油乐队', album: '甜点', lyrics: const []),
  );
}

Playlist _playlist() {
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

LyricsSessionSnapshot _lyricsSession({
  String targetPath = '/music/a.mp3',
  BigInt? previewId,
}) {
  return LyricsSessionSnapshot(
    sessionId: BigInt.one,
    targetPath: targetPath,
    status: LyricsSearchStatus.ready,
    previewId: previewId,
    candidates: [
      LyricsCandidateSnapshot(
        id: BigInt.one,
        provider: 'lrclib',
        providerLabel: 'lrclib',
        title: '候选一',
        artist: '奶油乐队',
        durationMs: BigInt.from(180000),
        synced: true,
      ),
      LyricsCandidateSnapshot(
        id: BigInt.two,
        provider: 'megalobiz',
        providerLabel: 'megalobiz',
        title: '候选二',
        artist: '奶油乐队',
        durationMs: BigInt.from(181000),
        synced: true,
      ),
    ],
  );
}

EffectiveLyrics _previewEffective() {
  return EffectiveLyrics(
    previewId: BigInt.one,
    content: LyricsContent(
      kind: LyricsContentKind.timed,
      lines: [
        LyricsLine(startMs: BigInt.zero, text: '预览第一句'),
        LyricsLine(startMs: BigInt.from(2000), text: '预览第二句'),
      ],
    ),
  );
}

PlayerSnapshot _playingSnapshot({
  LyricsSessionSnapshot? lyricsSession,
  EffectiveLyrics? effectiveLyrics,
}) {
  return PlayerSnapshot(
    kind: PlayerSnapshotKind.full,
    positionMs: BigInt.from(2500),
    durationMs: BigInt.from(180000),
    state: PlayerState.playing,
    volume: 0.8,
    playbackMode: PlaybackMode.sequential,
    currentIndex: BigInt.zero,
    currentFromUpNext: false,
    currentPath: '/music/a.mp3',
    displayTitle: '草莓糖',
    activeLyricsLine: BigInt.one,
    lyricsSession: lyricsSession,
    effectiveLyrics: effectiveLyrics,
    sourcePlaylistId: BigInt.one,
    queue: [
      QueueItem(index: BigInt.zero, path: '/music/a.mp3'),
      QueueItem(index: BigInt.one, path: '/music/b.mp3'),
    ],
    upNext: const [],
    metadata: _meta(lyrics: _timedLyrics()),
  );
}

Widget _bannered(Widget child) {
  return Directionality(
    textDirection: TextDirection.ltr,
    child: Stack(
      children: [
        child,
        Positioned(
          top: 0,
          left: 0,
          right: 0,
          child: IgnorePointer(
            child: ColoredBox(
              color: const Color(0xCC7A1F1F),
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: 12,
                  vertical: 6,
                ),
                child: Text(
                  _bannerText,
                  textAlign: TextAlign.center,
                  style: const TextStyle(
                    color: Colors.white,
                    fontSize: 13,
                    decoration: TextDecoration.none,
                    fontWeight: FontWeight.w600,
                  ),
                ),
              ),
            ),
          ),
        ),
      ],
    ),
  );
}

Future<void> _prepareView(WidgetTester tester, Size size) async {
  tester.view.physicalSize = size;
  tester.view.devicePixelRatio = 1;
  addTearDown(() {
    tester.view.resetPhysicalSize();
    tester.view.resetDevicePixelRatio();
  });
}

Future<void> _pumpHome(
  WidgetTester tester, {
  required PlayerController player,
  required Size size,
}) async {
  await _prepareView(tester, size);
  final collection = CollectionController.fake(
    playlists: [_playlist()],
    selectedId: BigInt.one,
  );
  await tester.pumpWidget(
    RepaintBoundary(
      key: const ValueKey('lyrics-layout-preview'),
      child: _bannered(
        MofuApp(
          collectionController: collection,
          playerController: player,
          settingsController: SettingsController.fake(),
        ),
      ),
    ),
  );
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 50));
}

Future<void> _pumpNowPlaying(
  WidgetTester tester, {
  required PlayerController player,
  required Size size,
}) async {
  await _prepareView(tester, size);
  await tester.pumpWidget(
    RepaintBoundary(
      key: const ValueKey('lyrics-layout-preview'),
      child: _bannered(
        MultiProvider(
          providers: [
            ChangeNotifierProvider<PlayerController>.value(value: player),
            ChangeNotifierProvider<SettingsController>.value(
              value: SettingsController.fake(),
            ),
          ],
          child: MaterialApp(
            theme: StrawberryCream.theme(),
            home: const NowPlayingPage(),
          ),
        ),
      ),
    ),
  );
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 50));
}

void _assertNoOverflow(WidgetTester tester) {
  final exception = tester.takeException();
  if (exception != null) {
    fail('unexpected Flutter exception (possible overflow): $exception');
  }
}

void _assertPanelDoesNotCoverBar(WidgetTester tester) {
  final bar = tester.getRect(find.byType(PlaybackBar));
  final panel = tester.getRect(find.byType(LyricsSessionPanel));
  expect(
    panel.bottom,
    lessThanOrEqualTo(bar.top + 0.5),
    reason:
        'LyricsSessionPanel (${panel.bottom}) must not cover PlaybackBar (${bar.top})',
  );
}

Future<void> _savePreview(WidgetTester tester, String filename) async {
  final out = File('$_previewDir/$filename');
  await tester.runAsync(() async {
    final boundary = tester.renderObject<RenderRepaintBoundary>(
      find.byKey(const ValueKey('lyrics-layout-preview')),
    );
    final image = await boundary.toImage(pixelRatio: 1);
    final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
    expect(bytes, isNotNull, reason: 'toImage produced no PNG bytes');
    out.parent.createSync(recursive: true);
    out.writeAsBytesSync(bytes!.buffer.asUint8List());
  });
  expect(out.existsSync(), isTrue);
  expect(out.lengthSync(), greaterThan(1000));
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  testWidgets('wide home: lyrics panel does not cover the playback bar', (
    tester,
  ) async {
    final player = PlayerController.fake(
      snapshot: _playingSnapshot(
        lyricsSession: _lyricsSession(previewId: BigInt.one),
        effectiveLyrics: _previewEffective(),
      ),
    );
    await _pumpHome(tester, player: player, size: const Size(1200, 800));
    expect(find.text(_bannerText), findsOneWidget);
    expect(find.byType(LyricsSessionPanel), findsOneWidget);
    expect(find.byType(PlaybackBar), findsOneWidget);
    _assertPanelDoesNotCoverBar(tester);
    _assertNoOverflow(tester);
    await _savePreview(tester, 'home-wide-lyrics-panel.png');
  });

  testWidgets('narrow home: lyrics panel does not overflow or cover the bar', (
    tester,
  ) async {
    final player = PlayerController.fake(
      snapshot: _playingSnapshot(
        lyricsSession: _lyricsSession(previewId: BigInt.one),
        effectiveLyrics: _previewEffective(),
      ),
    );
    await _pumpHome(tester, player: player, size: const Size(700, 900));
    expect(find.text(_bannerText), findsOneWidget);
    expect(find.byType(LyricsSessionPanel), findsOneWidget);
    expect(find.byType(PlaybackBar), findsOneWidget);
    _assertPanelDoesNotCoverBar(tester);
    _assertNoOverflow(tester);
    await _savePreview(tester, 'home-narrow-lyrics-panel.png');
  });

  testWidgets('now playing: preview lyrics and session panel render', (
    tester,
  ) async {
    final player = PlayerController.fake(
      snapshot: _playingSnapshot(
        lyricsSession: _lyricsSession(previewId: BigInt.one),
        effectiveLyrics: _previewEffective(),
      ),
    );
    await _pumpNowPlaying(tester, player: player, size: const Size(960, 800));
    expect(find.text(_bannerText), findsOneWidget);
    expect(find.byType(LyricsSessionPanel), findsOneWidget);
    expect(find.text('预览第一句'), findsWidgets);
    _assertNoOverflow(tester);
    await _savePreview(tester, 'now-playing-preview-panel.png');
  });
}
