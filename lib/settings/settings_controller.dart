import 'package:flutter/foundation.dart';
import 'package:mofu/src/rust/api/app.dart';

/// Thin UI adapter for app preferences persisted by Rust.
class SettingsController extends ChangeNotifier {
  SettingsController() : _fake = false;

  @visibleForTesting
  SettingsController.fake({
    this._coverAnimationInBackground = CoverAnimationInBackground.pause,
  }) : _fake = true;

  App? _app;
  final bool _fake;
  CoverAnimationInBackground _coverAnimationInBackground =
      CoverAnimationInBackground.pause;
  String? _error;
  bool _disposed = false;

  /// Commands recorded by the fake controller for widget tests.
  @visibleForTesting
  final List<String> recordedCommands = [];

  /// When set, invoked instead of the Rust setter (for rollback tests).
  @visibleForTesting
  void Function(CoverAnimationInBackground value)? testSetCoverAnimation;

  CoverAnimationInBackground get coverAnimationInBackground =>
      _coverAnimationInBackground;

  /// Whether the now-playing cover should stop spinning in the background.
  bool get pauseCoverAnimationInBackground =>
      _coverAnimationInBackground == CoverAnimationInBackground.pause;

  String? get error => _error;

  /// Bind to the shared Rust [App] with the bootstrap preference.
  void attach(App app, {CoverAnimationInBackground? initial}) {
    if (_fake || _disposed) {
      return;
    }
    _app = app;
    _coverAnimationInBackground = initial ?? app.coverAnimationInBackground();
    _error = null;
    notifyListeners();
  }

  void setCoverAnimationInBackground(CoverAnimationInBackground value) {
    if (_disposed || value == _coverAnimationInBackground) {
      return;
    }
    final previous = _coverAnimationInBackground;
    _coverAnimationInBackground = value;
    _error = null;
    notifyListeners();

    if (_fake) {
      recordedCommands.add('setCoverAnimationInBackground:$value');
      return;
    }

    final testSetter = testSetCoverAnimation;
    if (testSetter != null) {
      try {
        testSetter(value);
      } catch (err) {
        _coverAnimationInBackground = previous;
        _error = err.toString();
        notifyListeners();
      }
      return;
    }

    final app = _app;
    if (app == null) {
      _coverAnimationInBackground = previous;
      _error = '设置仍在初始化';
      notifyListeners();
      return;
    }

    try {
      app.setCoverAnimationInBackground(value: value);
    } catch (err) {
      _coverAnimationInBackground = previous;
      _error = err.toString();
      notifyListeners();
    }
  }

  void setPauseCoverAnimationInBackground(bool pause) {
    setCoverAnimationInBackground(
      pause
          ? CoverAnimationInBackground.pause
          : CoverAnimationInBackground.continue_,
    );
  }

  @override
  void dispose() {
    _disposed = true;
    super.dispose();
  }
}
