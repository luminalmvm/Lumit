// Replacing Lumit with a newer Lumit, from inside Lumit.
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
  });

  group('staging', () {
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

    test('the swap works from inside the folder being replaced', () {
      // **The v0.2 -> v0.3 upgrade that never happened**. Lumit's own
      // current directory is its install folder — Inno's shortcut takes
      // `WorkingDir` from `{app}` — and Windows will not rename a folder a
      // process is standing in. So the first rename threw, the update was
      // rolled back before it began, and the restart button appeared to do
      // nothing. Renaming a folder whose *executable* is running is fine and
      // always was; it is only the working directory that holds it.
      //
      // Fails on Windows without the step-out in `swapInStagedUpdate`. On the
      // other two it passes either way — they allow the rename — which is why
      // this test is worth having in CI's Windows job specifically.
      final site = install('0.1.0');
      stage(site, '0.2.0');
      final wasCurrent = Directory.current;
      Directory.current = site.root;
      try {
        swapInStagedUpdate(site);
      } finally {
        Directory.current = wasCurrent;
      }

      expect(versionAt(site), '0.2.0');
      expect(site.staging.existsSync(), isFalse);
    });
  });

  group('tidying up at start-up', () {
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
  });

  group('unwrapping an archive', () {
    test('an archive with one folder in it is that folder', () {
      final unpacked = Directory('${tmp.path}/unpacked')..createSync();
      Directory('${unpacked.path}/lumit-0.2.0-linux-x64').createSync();
      expect(unwrapSingleFolder(unpacked).path, endsWith('linux-x64'));
    });

    test('one folder beside a loose file is not a wrapper', () {
      final unpacked = Directory('${tmp.path}/unpacked')..createSync();
      Directory('${unpacked.path}/data').createSync();
      File('${unpacked.path}/lumit.exe').writeAsStringSync('x');
      expect(unwrapSingleFolder(unpacked).path, unpacked.path);
    });
  });
}
