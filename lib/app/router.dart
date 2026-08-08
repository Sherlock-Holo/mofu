import 'package:go_router/go_router.dart';
import 'package:mofu/features/home/home_page.dart';
import 'package:mofu/features/now_playing/now_playing_page.dart';
import 'package:mofu/features/settings/settings_page.dart';

GoRouter createRouter() {
  return GoRouter(
    initialLocation: '/',
    routes: [
      GoRoute(
        path: '/',
        name: 'home',
        builder: (context, state) => const HomePage(),
      ),
      GoRoute(
        path: '/now-playing',
        name: 'now-playing',
        builder: (context, state) => const NowPlayingPage(),
      ),
      GoRoute(
        path: '/settings',
        name: 'settings',
        builder: (context, state) => const SettingsPage(),
      ),
    ],
  );
}
