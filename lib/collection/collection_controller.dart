import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:mofu/src/rust/api/app.dart';
import 'package:mofu/src/rust/api/collection.dart';

/// Thin UI adapter: holds selection state and forwards library commands to Rust.
class CollectionController extends ChangeNotifier {
  CollectionController() : _fake = false;

  @visibleForTesting
  CollectionController.fake({
    List<Playlist> playlists = const [],
    BigInt? selectedId,
  }) : _fake = true,
       _playlists = List<Playlist>.unmodifiable(playlists) {
    _selectedId = selectedId;
    _ensureSelection();
  }

  App? _app;
  StreamSubscription<LibrarySnapshot>? _subscription;
  final bool _fake;

  List<Playlist> _playlists = const [];
  BigInt? _selectedId;
  bool _loading = false;
  String? _error;
  VoidCallback? _retry;
  bool _disposed = false;

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

  List<Playlist> get directoryPlaylists => [
    for (final playlist in _playlists)
      if (playlist.kind == PlaylistKind.directory) playlist,
  ];

  List<Playlist> get virtualPlaylists => [
    for (final playlist in _playlists)
      if (playlist.kind == PlaylistKind.virtual) playlist,
  ];

  /// Bind to the shared Rust [App] and start the library stream.
  void attach(App app, {LibrarySnapshot? initial, BigInt? selectedPlaylistId}) {
    if (_fake || _disposed) {
      return;
    }
    _app = app;
    if (initial != null) {
      _playlists = List<Playlist>.unmodifiable(initial.playlists);
    }
    if (selectedPlaylistId != null) {
      _selectedId = selectedPlaylistId;
    }
    _ensureSelection();
    _subscribe();
    notifyListeners();
  }

  void _subscribe() {
    final app = _app;
    if (app == null || _disposed) {
      return;
    }
    unawaited(_subscription?.cancel());
    _subscription = app.watchLibrary().listen(
      (snapshot) {
        if (_disposed) {
          return;
        }
        _playlists = List<Playlist>.unmodifiable(snapshot.playlists);
        _ensureSelection();
        _error = null;
        _retry = null;
        notifyListeners();
      },
      onError: (Object err) {
        if (_disposed) {
          return;
        }
        _error = err.toString();
        _retry = _subscribe;
        notifyListeners();
      },
    );
  }

  @visibleForTesting
  void emitPlaylists(List<Playlist> playlists, {BigInt? selectedId}) {
    if (_disposed) {
      return;
    }
    _playlists = List<Playlist>.unmodifiable(playlists);
    if (selectedId != null) {
      _selectedId = selectedId;
    }
    _ensureSelection();
    _error = null;
    notifyListeners();
  }

  void selectPlaylist(BigInt id) {
    if (_disposed) {
      return;
    }
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

  Future<void> _mutate(Future<void> Function(App app) action) async {
    if (_fake || _disposed) {
      return;
    }
    final app = _app;
    if (app == null) {
      _error = '曲库仍在初始化';
      notifyListeners();
      return;
    }
    _loading = true;
    _error = null;
    notifyListeners();
    try {
      await action(app);
      if (!_disposed) {
        _retry = null;
      }
    } catch (err) {
      if (!_disposed) {
        _error = err.toString();
        _retry = () => unawaited(_mutate(action));
      }
    } finally {
      if (!_disposed) {
        _loading = false;
        notifyListeners();
      }
    }
  }

  Future<void> createVirtual(String name) async {
    final trimmed = name.trim();
    if (trimmed.isEmpty) {
      return;
    }
    if (_fake) {
      final id = BigInt.from(_playlists.length + 1);
      _playlists = [
        ..._playlists,
        Playlist(
          id: id,
          name: trimmed,
          kind: PlaylistKind.virtual,
          songs: const [],
        ),
      ];
      _selectedId = id;
      notifyListeners();
      return;
    }
    await _mutate((app) async {
      final id = await app.createVirtual(name: trimmed);
      _selectedId = id;
    });
  }

  Future<void> renameVirtual(BigInt id, String name) async {
    final trimmed = name.trim();
    if (trimmed.isEmpty) {
      return;
    }
    if (_fake) {
      _playlists = [
        for (final playlist in _playlists)
          if (playlist.id == id)
            Playlist(
              id: playlist.id,
              name: trimmed,
              kind: playlist.kind,
              directoryPath: playlist.directoryPath,
              cover: playlist.cover,
              songs: playlist.songs,
            )
          else
            playlist,
      ];
      notifyListeners();
      return;
    }
    await _mutate((app) => app.renameVirtual(id: id, name: trimmed));
  }

  Future<void> deleteVirtual(BigInt id) async {
    if (_fake) {
      _playlists = [
        for (final playlist in _playlists)
          if (playlist.id != id) playlist,
      ];
      if (_selectedId == id) {
        _selectedId = null;
      }
      _ensureSelection();
      notifyListeners();
      return;
    }
    await _mutate((app) async {
      await app.deleteVirtual(id: id);
      if (_selectedId == id) {
        _selectedId = null;
        for (final playlist in _playlists) {
          if (playlist.id != id) {
            _selectedId = playlist.id;
            break;
          }
        }
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
    if (_fake) {
      return;
    }
    await _mutate((app) => app.refresh(id: id));
  }

  Future<void> pickAndAddDirectory() async {
    if (_fake) {
      return;
    }
    await _mutate((app) async {
      final id = await app.pickAndAddDirectory();
      if (id != null) {
        _selectedId = id;
      }
    });
  }

  Future<void> pickAndAddFiles() async {
    if (_fake) {
      return;
    }
    await _mutate((app) async {
      final id = await app.pickAndAddFiles(selectedPlaylistId: _selectedId);
      if (id != null) {
        _selectedId = id;
      }
    });
  }

  Future<void> importFromPlaylist({
    required BigInt targetId,
    required BigInt sourceId,
    List<String> paths = const [],
  }) async {
    if (_fake) {
      return;
    }
    await _mutate(
      (app) => app.importFromPlaylist(
        targetId: targetId,
        sourceId: sourceId,
        paths: paths,
      ),
    );
  }

  void retryLastError() {
    if (_disposed) {
      return;
    }
    _retry?.call();
  }

  @override
  void dispose() {
    _disposed = true;
    _subscription?.cancel();
    _subscription = null;
    _app = null;
    _retry = null;
    super.dispose();
  }
}
