
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
}
