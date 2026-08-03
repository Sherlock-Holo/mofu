import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:mofu/src/rust/api/file_dialog.dart';
import 'package:mofu/src/rust/api/library.dart';
import 'package:mofu/src/rust/api/player.dart'
    show CoverArt, PlaybackMode, TrackMetadata;

/// 曲库列表状态与目录/文件导入动作。
class LibraryController extends ChangeNotifier {
  LibraryController({Library? library})
      : _library = library,
        _ownsLibrary = library == null,
        _fake = false;

  /// 不连接 Rust，供 Widget 与 integration 测试注入。
  @visibleForTesting
  LibraryController.fake({
    List<Playlist> playlists = const [],
    this._selectedId,
  })  : _library = null,
        _ownsLibrary = false,
        _fake = true,
       _playlists = List<Playlist>.unmodifiable(playlists) {
    _ensureSelection();
  }

  Library? _library;
  final bool _ownsLibrary;
  final bool _fake;

  List<Playlist> _playlists = const [];
  BigInt? _selectedId;
  bool _loading = false;
  String? _error;

  List<Playlist> get playlists => _playlists;
  BigInt? get selectedId => _selectedId;
  bool get loading => _loading;
  String? get error => _error;

  Playlist? get selectedPlaylist {
    final id = _selectedId;
    if (id == null) {
      return null;
    }
    for (final playlist in _playlists) {
      if (playlist.id == id) {
        return playlist;
      }
    }
    return null;
  }

  List<Playlist> get directoryPlaylists =>
      _playlists.where((p) => p.kind == PlaylistKind.directory).toList();

  List<Playlist> get virtualPlaylists =>
      _playlists.where((p) => p.kind == PlaylistKind.virtual).toList();

  /// 测试用：替换曲库快照。
  @visibleForTesting
  void emitPlaylists(List<Playlist> playlists, {BigInt? selectedId}) {
    _playlists = List<Playlist>.unmodifiable(playlists);
    if (selectedId != null) {
      _selectedId = selectedId;
    }
    _ensureSelection();
    _error = null;
    notifyListeners();
  }

  Future<Library> _requireLibrary() async {
    return _library ??= await Library.newInstance();
  }

  /// 单次 FFI 封面批量上限，避免巨大同步 payload。
  static const int _coverHydrateChunkSize = 32;

  Future<void> load() async {
    if (_fake) {
      _loading = false;
      _error = null;
      _ensureSelection();
      notifyListeners();
      return;
    }
    _loading = true;
    _error = null;
    notifyListeners();
    try {
      final library = await _requireLibrary();
      _playlists = await library.playlists();
      _ensureSelection();
    } catch (err) {
      _error = err.toString();
    } finally {
      _loading = false;
      notifyListeners();
    }
    if (_error == null) {
      unawaited(hydrateCovers());
    }
  }

  /// 在 lite 歌单已展示后，按 path 分块补封面。
  Future<void> hydrateCovers() async {
    if (_fake || _playlists.isEmpty) {
      return;
    }
    final paths = <String>[
      for (final playlist in _playlists)
        for (final song in playlist.songs)
          if (song.metadata.cover == null) song.path,
    ];
    if (paths.isEmpty) {
      return;
    }
    try {
      final library = await _requireLibrary();
      final byPath = <String, CoverArt>{};
      for (var i = 0; i < paths.length; i += _coverHydrateChunkSize) {
        final end = (i + _coverHydrateChunkSize).clamp(0, paths.length);
        final chunk = paths.sublist(i, end);
        final covers = await library.coversForPaths(paths: chunk);
        for (final item in covers) {
          byPath[item.path] = item.cover;
        }
      }
      if (byPath.isEmpty) {
        return;
      }
      _playlists = [
        for (final playlist in _playlists)
          Playlist(
            id: playlist.id,
            name: playlist.name,
            kind: playlist.kind,
            directoryPath: playlist.directoryPath,
            songs: [
              for (final song in playlist.songs)
                _songWithCover(song, byPath[song.path]),
            ],
          ),
      ];
      notifyListeners();
    } catch (_) {
      // 封面补全失败不影响列表文字；保留 placeholder。
    }
  }

  Song _songWithCover(Song song, CoverArt? cover) {
    if (cover == null) {
      return song;
    }
    final meta = song.metadata;
    return Song(
      path: song.path,
      durationMs: song.durationMs,
      metadata: TrackMetadata(
        title: meta.title,
        artist: meta.artist,
        album: meta.album,
        albumArtist: meta.albumArtist,
        composer: meta.composer,
        lyricist: meta.lyricist,
        performer: meta.performer,
        genre: meta.genre,
        comment: meta.comment,
        date: meta.date,
        year: meta.year,
        trackNumber: meta.trackNumber,
        trackTotal: meta.trackTotal,
        discNumber: meta.discNumber,
        discTotal: meta.discTotal,
        cover: cover,
        lyrics: meta.lyrics,
      ),
    );
  }

  /// 读取持久化的播放模式；缺省为顺序播放。
  Future<PlaybackMode> loadPlaybackMode() async {
    if (_fake) {
      return PlaybackMode.sequential;
    }
    final library = await _requireLibrary();
    return library.playbackMode();
  }

  /// 将播放模式写入 catalog.db。
  Future<void> persistPlaybackMode(PlaybackMode mode) async {
    if (_fake) {
      return;
    }
    final library = await _requireLibrary();
    await library.setPlaybackMode(mode: mode);
  }

  /// 读取持久化音量；缺省为 `1.0`。
  Future<double> loadVolume() async {
    if (_fake) {
      return 1;
    }
    final library = await _requireLibrary();
    return library.volume();
  }

  /// 将音量写入 catalog.db（已在 Rust 侧 clamp 到 `[0, 1]`）。
  Future<void> persistVolume(double volume) async {
    if (_fake) {
      return;
    }
    final library = await _requireLibrary();
    await library.setVolume(volume: volume);
  }

  /// 读取持久化播放会话；无会话时为 null。
  Future<PlaybackSession?> loadPlaybackSession() async {
    if (_fake) {
      return null;
    }
    final library = await _requireLibrary();
    return library.playbackSession();
  }

  /// 将当前播放会话写入 catalog.db。
  Future<void> persistPlaybackSession(PlaybackSession session) async {
    if (_fake) {
      return;
    }
    final library = await _requireLibrary();
    await library.setPlaybackSession(session: session);
  }

  /// 清除持久化播放会话。
  Future<void> clearPlaybackSession() async {
    if (_fake) {
      return;
    }
    final library = await _requireLibrary();
    await library.clearPlaybackSession();
  }

  void selectPlaylist(BigInt id) {
    _selectedId = id;
    notifyListeners();
  }

  void _ensureSelection() {
    if (_playlists.isEmpty) {
      _selectedId = null;
      return;
    }
    if (_selectedId != null &&
        _playlists.any((playlist) => playlist.id == _selectedId)) {
      return;
    }
    _selectedId = _playlists.first.id;
  }

  Future<void> _mutate(Future<void> Function() action) async {
    _loading = true;
    _error = null;
    notifyListeners();
    try {
      await action();
      if (!_fake) {
        final library = await _requireLibrary();
        _playlists = await library.playlists();
      }
      _ensureSelection();
    } catch (err) {
      _error = err.toString();
    } finally {
      _loading = false;
      notifyListeners();
    }
    if (!_fake && _error == null) {
      unawaited(hydrateCovers());
    }
  }

  Future<void> createVirtual(String name) async {
    final trimmed = name.trim();
    if (trimmed.isEmpty) {
      return;
    }
    await _mutate(() async {
      if (_fake) {
        final id = BigInt.from(_playlists.length + 1);
        final created = Playlist(
          id: id,
          name: trimmed,
          kind: PlaylistKind.virtual,
          songs: const [],
        );
        _playlists = [..._playlists, created];
        _selectedId = created.id;
        return;
      }
      final library = await _requireLibrary();
      final created = await library.createVirtual(name: trimmed);
      _selectedId = created.id;
    });
  }

  Future<void> renameVirtual(BigInt id, String name) async {
    final trimmed = name.trim();
    if (trimmed.isEmpty) {
      return;
    }
    await _mutate(() async {
      if (_fake) {
        _playlists = [
          for (final playlist in _playlists)
            if (playlist.id == id)
              Playlist(
                id: playlist.id,
                name: trimmed,
                kind: playlist.kind,
                directoryPath: playlist.directoryPath,
                songs: playlist.songs,
              )
            else
              playlist,
        ];
        return;
      }
      final library = await _requireLibrary();
      await library.renameVirtual(id: id, name: trimmed);
    });
  }

  Future<void> deleteVirtual(BigInt id) async {
    await _mutate(() async {
      if (_fake) {
        _playlists = [
          for (final playlist in _playlists)
            if (playlist.id != id) playlist,
        ];
        if (_selectedId == id) {
          _selectedId = null;
        }
        return;
      }
      final library = await _requireLibrary();
      await library.deleteVirtual(id: id);
      if (_selectedId == id) {
        _selectedId = null;
      }
    });
  }

  Future<void> refreshSelected() async {
    final id = _selectedId;
    if (id == null) {
      return;
    }
    await refresh(id);
  }

  Future<void> refresh(BigInt id) async {
    await _mutate(() async {
      if (_fake) {
        return;
      }
      final library = await _requireLibrary();
      await library.refresh(id: id);
    });
  }

  /// 打开系统目录选择器并扫描为目录列表。
  Future<void> pickAndAddDirectory() async {
    if (_fake) {
      return;
    }
    final path = await pickMusicDirectory();
    if (path == null || path.isEmpty) {
      return;
    }
    await _mutate(() async {
      final library = await _requireLibrary();
      final created = await library.addDirectory(path: path);
      _selectedId = created.id;
    });
  }

  /// 多选音频文件加入当前虚拟列表；若无选中虚拟列表则先创建。
  Future<void> pickAndAddFiles() async {
    if (_fake) {
      return;
    }
    final paths = await pickMusicFiles();
    if (paths == null || paths.isEmpty) {
      return;
    }

    await _mutate(() async {
      final library = await _requireLibrary();
      var target = selectedPlaylist;
      if (target == null || target.kind != PlaylistKind.virtual) {
        target = await library.createVirtual(name: '导入的歌曲');
        _selectedId = target.id;
      }
      await library.addFiles(id: target.id, paths: paths);
    });
  }

  Future<void> importFromPlaylist({
    required BigInt targetId,
    required BigInt sourceId,
    List<String> paths = const [],
  }) async {
    await _mutate(() async {
      if (_fake) {
        Playlist? source;
        for (final playlist in _playlists) {
          if (playlist.id == sourceId) {
            source = playlist;
            break;
          }
        }
        if (source == null) {
          return;
        }
        final imported = paths.isEmpty
            ? source.songs
            : source.songs.where((song) => paths.contains(song.path)).toList();
        _playlists = [
          for (final playlist in _playlists)
            if (playlist.id == targetId)
              Playlist(
                id: playlist.id,
                name: playlist.name,
                kind: playlist.kind,
                directoryPath: playlist.directoryPath,
                songs: _mergeSongs(playlist.songs, imported),
              )
            else
              playlist,
        ];
        return;
      }
      final library = await _requireLibrary();
      await library.importFromPlaylist(
        targetId: targetId,
        sourceId: sourceId,
        paths: paths,
      );
    });
  }

  List<Song> _mergeSongs(List<Song> existing, List<Song> incoming) {
    final seen = existing.map((song) => song.path).toSet();
    return [
      ...existing,
      for (final song in incoming)
        if (seen.add(song.path)) song,
    ];
  }

  @override
  void dispose() {
    if (_ownsLibrary) {
      _library?.dispose();
    }
    super.dispose();
  }
}
