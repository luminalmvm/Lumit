// Replacing Lumit with a newer Lumit, from inside Lumit (K-297).
//
// This is the part of updating that can destroy an installation if it is wrong,
// so it is tested against real folders on disk rather than a pretend
// filesystem: the swap is two renames, and whether a rename does what this code
// assumes is a question only a real filesystem can answer.
//
// Every test builds a little installation in a temporary folder — a launcher, a
// library, an assets folder — and checks what is standing afterwards.

import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/install_site.dart';

void main() {
  late Directory tmp;

  setUp(() => tmp = Directory.systemTemp.createTempSync('lumit-install'));
  tearDown(() {
    if (tmp.existsSync()) tmp.deleteSync(recursive: true);
  });

  /// An installation at `<tmp>/Lumit`, with [version] written into every file
  /// so a test can say which one is standing.
  InstallSite install(String version) {
    final root = Directory('${tmp.path}/Lumit')..createSync(recursive: true);
    File('${root.path}/lumit_flutter').writeAsStringSync(version);
    File('${root.path}/liblumit_bridge.so').writeAsStringSync(version);
    Directory('${root.path}/data').createSync();
    File('${root.path}/data/icudtl.dat').writeAsStringSync(version);
    return InstallSite(
      kind: InstallKind.folder,
      root: root,
      launcher: File('${root.path}/lumit_flutter'),
    );
  }

  /// A complete staged update at `<tmp>/Lumit.new`.
  void stage(InstallSite site, String version, {bool complete = true}) {
    final staging = site.staging..createSync(recursive: true);
    File('${staging.path}/lumit_flutter').writeAsStringSync(version);
    File('${staging.path}/liblumit_bridge.so').writeAsStringSync(version);
    if (complete) markStagedUpdateReady(site);
  }

  String versionAt(InstallSite site) =>
      File('${site.root.path}/lumit_flutter').readAsStringSync();

  group('working out where we are', () {
    test('Windows and Linux are a folder of files', () {
      final site = InstallSite.detect(
        executablePath: r'C:\Users\me\AppData\Local\Programs\Lumit\lumit.exe',
        operatingSystem: 'windows',
      );
      expect(site.kind, InstallKind.folder);
      expect(site.root.path, endsWith('Lumit'));
      expect(site.launcher.path, endsWith('lumit.exe'));
    });

    test('macOS is the bundle, not the folder the binary sits in', () {
      final site = InstallSite.detect(
        executablePath: '/Applications/Lumit.app/Contents/MacOS/Lumit',
        operatingSystem: 'macos',
      );
      expect(site.kind, InstallKind.bundle);
      expect(site.root.path, '/Applications/Lumit.app',
          reason: 'the whole bundle is what gets swapped');
      expect(site.launcher.path, '/Applications/Lumit.app/Contents/MacOS/Lumit');
    });

    test('a macOS binary outside a bundle is not something to swap', () {
      final site = InstallSite.detect(
        executablePath: '/Users/me/build/lumit_flutter',
        operatingSystem: 'macos',
      );
      expect(site.kind, InstallKind.unknown);
      expect(site.replaceable, isFalse);
    });

    test('a Flatpak says so, and is never replaced from inside', () {
      final site = InstallSite.detect(
        executablePath: '/app/bin/lumit_flutter',
        operatingSystem: 'linux',
        isFlatpak: (path) => path == '/.flatpak-info',
      );
      expect(site.kind, InstallKind.flatpak);
      expect(site.replaceable, isFalse,
          reason: 'the sandbox is read-only by design');
    });

    test('a folder Lumit can write beside is replaceable, one it cannot is not',
        () {
      expect(install('0.1.0').replaceable, isTrue);
      final nowhere = InstallSite(
        kind: InstallKind.folder,
        root: Directory('/definitely/not/a/place/Lumit'),
        launcher: File('/definitely/not/a/place/Lumit/lumit'),
      );
      expect(nowhere.replaceable, isFalse);
    });
  });

  group('staging', () {
    test('a tree is only ready once it has been marked', () {
      final site = install('0.1.0');
      stage(site, '0.2.0', complete: false);
      expect(stagedUpdateReady(site), isFalse,
          reason: 'a half-unpacked download must never be swapped in');
      markStagedUpdateReady(site);
      expect(stagedUpdateReady(site), isTrue);
    });

    test('swapping refuses a tree that was never marked complete', () {
      final site = install('0.1.0');
      stage(site, '0.2.0', complete: false);
      expect(() => swapInStagedUpdate(site), throwsStateError);
      expect(versionAt(site), '0.1.0', reason: 'the old version is untouched');
    });
  });

  group('the swap', () {
    test('the new version takes the old one\'s place, at the same path', () {
      final site = install('0.1.0');
      final where = site.root.path;
      stage(site, '0.2.0');

      swapInStagedUpdate(site);

      expect(site.root.path, where,
          reason: 'shortcuts and file associations point at this path');
      expect(versionAt(site), '0.2.0');
      expect(site.staging.existsSync(), isFalse);
    });

    test('the old version is kept, because its files are still open', () {
      final site = install('0.1.0');
      stage(site, '0.2.0');
      swapInStagedUpdate(site);

      expect(site.previous.existsSync(), isTrue);
      expect(File('${site.previous.path}/lumit_flutter').readAsStringSync(),
          '0.1.0');
    });

    test('a version before last is cleared out of the way first', () {
      final site = install('0.1.0');
      site.previous.createSync();
      File('${site.previous.path}/lumit_flutter').writeAsStringSync('0.0.9');
      stage(site, '0.2.0');

      swapInStagedUpdate(site);

      expect(versionAt(site), '0.2.0');
      expect(File('${site.previous.path}/lumit_flutter').readAsStringSync(),
          '0.1.0', reason: 'the one just replaced, not the ancient one');
    });
  });

  group('tidying up at start-up', () {
    test('the replaced version is deleted once nothing holds it', () {
      final site = install('0.2.0');
      site.previous.createSync();
      File('${site.previous.path}/lumit_flutter').writeAsStringSync('0.1.0');

      tidyAfterUpdate(site);

      expect(site.previous.existsSync(), isFalse);
      expect(versionAt(site), '0.2.0', reason: 'the running version stands');
    });

    test('a swap cut in half is put back', () {
      final site = install('0.1.0');
      // What a power cut between the two renames leaves: the old version under
      // its safety name, and nothing at all where Lumit should be.
      site.root.renameSync(site.previous.path);
      expect(site.root.existsSync(), isFalse);

      tidyAfterUpdate(site);

      expect(site.root.existsSync(), isTrue);
      expect(versionAt(site), '0.1.0',
          reason: 'better the version they had than no Lumit at all');
      expect(site.previous.existsSync(), isFalse);
    });

    test('an abandoned download is swept, a complete one is left alone', () {
      final site = install('0.1.0');
      stage(site, '0.2.0', complete: false);
      tidyAfterUpdate(site);
      expect(site.staging.existsSync(), isFalse);

      stage(site, '0.2.0');
      tidyAfterUpdate(site);
      expect(site.staging.existsSync(), isTrue,
          reason: 'an update that is ready survives a restart that did not '
              'apply it');
    });

    test('leftover unpacking is swept', () {
      final site = install('0.1.0');
      site.unpacking.createSync();
      File('${site.unpacking.path}/half-a-file').writeAsStringSync('...');
      tidyAfterUpdate(site);
      expect(site.unpacking.existsSync(), isFalse);
    });
  });

  group('unwrapping an archive', () {
    test('an archive with one folder in it is that folder', () {
      final unpacked = Directory('${tmp.path}/unpacked')..createSync();
      Directory('${unpacked.path}/lumit-0.2.0-linux-x64').createSync();
      expect(unwrapSingleFolder(unpacked).path, endsWith('linux-x64'));
    });

    test('an archive of loose files is itself', () {
      final unpacked = Directory('${tmp.path}/unpacked')..createSync();
      File('${unpacked.path}/lumit.exe').writeAsStringSync('x');
      File('${unpacked.path}/lumit_bridge.dll').writeAsStringSync('x');
      expect(unwrapSingleFolder(unpacked).path, unpacked.path);
    });

    test('one folder beside a loose file is not a wrapper', () {
      final unpacked = Directory('${tmp.path}/unpacked')..createSync();
      Directory('${unpacked.path}/data').createSync();
      File('${unpacked.path}/lumit.exe').writeAsStringSync('x');
      expect(unwrapSingleFolder(unpacked).path, unpacked.path);
    });
  });
}
