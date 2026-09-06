import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/cache_dir.dart';

void main() {
  test('each platform gets the folder the engine names', () {
    expect(
      lumitCacheDir(
        platform: 'windows',
        env: const {'LOCALAPPDATA': r'C:\Users\a\AppData\Local'},
      ).path,
      r'C:\Users\a\AppData\Local\Lumit\Lumit\cache',
    );
    expect(
      lumitCacheDir(platform: 'macos', env: const {'HOME': '/Users/a'}).path,
      '/Users/a/Library/Caches/dev.Lumit.Lumit',
    );
    expect(
      lumitCacheDir(platform: 'linux', env: const {'HOME': '/home/a'}).path,
      '/home/a/.cache/lumit',
    );
  });

  test('a Flatpak names its own cache through XDG_CACHE_HOME', () {
    expect(
      lumitCacheDir(platform: 'linux', env: const {
        'HOME': '/home/a',
        'XDG_CACHE_HOME': '/home/a/.var/app/io.github.luminalmvm.Lumit/cache',
      }).path,
      '/home/a/.var/app/io.github.luminalmvm.Lumit/cache/lumit',
    );
    expect(
      lumitCacheDir(platform: 'linux', env: const {
        'HOME': '/home/a',
        'XDG_CACHE_HOME': 'relative/cache',
      }).path,
      '/home/a/.cache/lumit',
    );
  });

  test('no home at all falls back to the temporary folder, not a crash', () {
    expect(
      lumitCacheDir(platform: 'linux', env: const {}).path,
      startsWith(Directory.systemTemp.path),
    );
  });
}
