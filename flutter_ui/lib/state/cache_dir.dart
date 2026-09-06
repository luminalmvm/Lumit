// Lumit's cache folder, the same one the engine names in
// `lumit_project::cache_dir`, worked out without the bridge so the fault log
// and the updater still work when the engine is the thing that broke.

import 'dart:io';

/// Lumit's cache folder as the engine's `ProjectDirs` names it:
/// `%LOCALAPPDATA%\Lumit\Lumit\cache`, `~/Library/Caches/dev.Lumit.Lumit` or
/// `$XDG_CACHE_HOME/lumit` (default `~/.cache/lumit`), with the system
/// temporary folder only when there is no home at all.
Directory lumitCacheDir({String? platform, Map<String, String>? env}) {
  final os = platform ?? Platform.operatingSystem;
  final vars = env ?? Platform.environment;
  String? base;
  switch (os) {
    case 'windows':
      final local = vars['LOCALAPPDATA'];
      if (local != null) base = '$local\\Lumit\\Lumit\\cache';
    case 'macos':
      final home = vars['HOME'];
      if (home != null) base = '$home/Library/Caches/dev.Lumit.Lumit';
    default:
      // The engine's `directories` crate ignores a relative XDG_CACHE_HOME,
      // so this does too.
      final xdg = vars['XDG_CACHE_HOME'];
      final home = vars['HOME'];
      if (xdg != null && xdg.startsWith('/')) {
        base = '$xdg/lumit';
      } else if (home != null) {
        base = '$home/.cache/lumit';
      }
  }
  return Directory(
      base ?? '${Directory.systemTemp.path}${Platform.pathSeparator}lumit');
}
