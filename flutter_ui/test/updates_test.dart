// Finding, fetching and applying a newer Lumit.
//
// Every seam the updater has with the outside world is injected, so this suite
// never reaches the network, never writes an installer anywhere but a scratch
// folder, and — the one that matters — never runs one and never quits the
// process. What it does exercise is the whole sequence: the version arithmetic
// that decides whether there *is* an update, the state machine the Help menu
// row reads its wording from, the checks that stand between a download and
// something being executed, and the two windows at the end of it.

import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/update_dialog_frb.dart';
import 'package:lumit_flutter/state/install_site.dart';
import 'package:lumit_flutter/state/updates.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

void main() {
  group('version arithmetic', () {
    test('newer, older and the same', () {
      expect(compareVersions('0.2.0', '0.1.0'), greaterThan(0));
      expect(compareVersions('0.1.0', '0.2.0'), lessThan(0));
      expect(compareVersions('0.1.0', '0.1.0'), 0);
      // Ten is not one: the numbers are compared as numbers, which a string
      // comparison would get backwards.
      expect(compareVersions('0.10.0', '0.9.0'), greaterThan(0));
      // A missing place is zero.
      expect(compareVersions('1.2', '1.2.0'), 0);
    });
  });

  group('reading a release', () {
    test('each platform is offered the attachment it can install', () {
      for (final (platform, expected) in const [
        ('windows', 'lumit-0.2.0-windows-x64-setup.exe'),
        ('macos', 'lumit-0.2.0.dmg'),
        ('linux', 'lumit-0.2.0-linux-x64.tar.gz'),
      ]) {
        final release = UpdateRelease.parse(_releaseJson(), platform: platform);
        expect(release?.assetName, expected, reason: platform);
        expect(release?.version, '0.2.0');
      }
    });

    test('drafts and pre-releases are refused', () {
      expect(
        UpdateRelease.parse(_releaseJson(draft: true), platform: 'windows'),
        isNull,
      );
      expect(
        UpdateRelease.parse(_releaseJson(prerelease: true),
            platform: 'windows'),
        isNull,
      );
    });

    test('an attachment whose name is not a bare file name is never offered',
        () {
      // The name ends up in a local path, so anything that could leave the
      // download folder is refused before the suffix is even looked at.
      for (final name in const [
        '../evil.exe',
        r'..\..\evil.exe',
        '/tmp/evil.tar.gz',
        r'C:\Temp\evil.exe',
        'lumit/0.2.0-windows-x64-setup.exe',
        '.exe',
      ]) {
        expect(isPlainFileName(name), isFalse, reason: name);
        final json = _releaseJson(assets: [_asset(name)]);
        for (final platform in const ['windows', 'macos', 'linux']) {
          expect(UpdateRelease.parse(json, platform: platform), isNull,
              reason: '$name on $platform');
        }
      }
      expect(isPlainFileName('lumit-0.2.0-windows-x64-setup.exe'), isTrue);
      expect(isPlainFileName('Lumit-0.2.0-rc1_macos-arm64.zip'), isTrue);
    });
  });

  group('checking', () {
    test('a newer release is offered by version, in the row itself', () async {
      final service = _service(fetch: (_) async => _releaseJson());
      await service.check();
      expect(service.stage, UpdateStage.available);
      // The exact wording the owner asked for. A test rather than a comment,
      // because this string is the whole feature as far as anyone using Lumit
      // is concerned.
      expect(service.menuLabel, 'Click to update - v0.2.0');
      expect(service.busy, isFalse);
    });

    test('no network is a sentence, not a crash', () async {
      final service =
          _service(fetch: (_) async => throw const SocketException('no route'));
      await service.check();
      expect(service.stage, UpdateStage.failed);
      expect(service.failure, 'Could not check for updates');
      // Recoverable: the row offers the check again.
      expect(service.menuLabel, 'Check for updates');
    });
  });

  group('downloading', () {
    late Directory scratch;

    setUp(() => scratch = Directory.systemTemp.createTempSync('lumit-update'));
    tearDown(() {
      if (scratch.existsSync()) scratch.deleteSync(recursive: true);
    });

    /// A service whose download writes [body] to the file it is given.
    UpdateService serviceFor(List<int> body, {String? digest}) => _service(
          fetch: (_) async => _releaseJson(assets: [
            _asset('lumit-0.2.0-windows-x64-setup.exe',
                size: body.length, digest: digest),
          ]),
          folder: () => scratch,
          download: (url, into,
              {required onProgress, required cancelled}) async {
            into.writeAsBytesSync(body);
            onProgress(body.length, body.length);
          },
        );

    test('a sound download ends ready, on disk, with the restart wording',
        () async {
      final body = utf8.encode('an installer, for the sake of argument');
      final service =
          serviceFor(body, digest: 'sha256:${sha256.convert(body)}');
      await service.check();
      await service.downloadUpdate();

      expect(service.stage, UpdateStage.ready);
      expect(service.menuLabel, 'Restart to finish updating');
      expect(service.downloadedInstaller?.existsSync(), isTrue);
      expect(service.progress, 1);
    });

    test('a short file is not run, and does not stay on disk', () async {
      final service = _service(
        fetch: (_) async => _releaseJson(assets: [
          _asset('lumit-0.2.0-windows-x64-setup.exe', size: 999),
        ]),
        folder: () => scratch,
        download: (url, into, {required onProgress, required cancelled}) async {
          into.writeAsBytesSync(utf8.encode('truncated'));
        },
      );
      await service.check();
      await service.downloadUpdate();

      expect(service.stage, UpdateStage.failed);
      expect(service.failure, 'The downloaded update was incomplete');
      expect(scratch.listSync(), isEmpty);
    });

    test('a file that does not match its checksum is not run', () async {
      final body = utf8.encode('an installer, or is it');
      final service = serviceFor(body, digest: 'sha256:${'0' * 64}');
      await service.check();
      await service.downloadUpdate();

      expect(service.stage, UpdateStage.failed);
      expect(
          service.failure,
          'The downloaded update did not match its '
          'checksum');
      expect(scratch.listSync(), isEmpty);
    });

    test('cancelling leaves the offer standing', () async {
      final service = _service(
        fetch: (_) async => _releaseJson(),
        folder: () => scratch,
        download: (url, into, {required onProgress, required cancelled}) async {
          // A turn of the loop, so the Cancel below lands mid-download the way
          // a real one does — the service asks as each chunk arrives.
          await Future<void>.delayed(Duration.zero);
          expect(cancelled(), isTrue);
          into.writeAsBytesSync(utf8.encode('half of one'));
        },
      );
      await service.check();
      final fetching = service.downloadUpdate();
      service.cancelDownload();
      await fetching;

      expect(service.stage, UpdateStage.available);
      expect(service.menuLabel, 'Click to update - v0.2.0');
      expect(scratch.listSync(), isEmpty);
    });
  });

  group('installing', () {
    late Directory scratch;
    setUp(() => scratch = Directory.systemTemp.createTempSync('lumit-update'));
    tearDown(() {
      if (scratch.existsSync()) scratch.deleteSync(recursive: true);
    });

    test('a Flatpak is handed its bundle and Lumit stays open', () async {
      final launched = <File>[];
      final quits = <int>[];
      final body = utf8.encode('bundle');
      final service = _service(
        platform: 'linux',
        site: InstallSite(
          kind: InstallKind.flatpak,
          root: Directory('/app'),
          launcher: File('/app/bin/lumit_flutter'),
        ),
        folder: () => scratch,
        fetch: (_) async => _releaseJson(assets: [
          _asset('lumit-0.2.0-linux-x64.tar.gz', size: body.length),
          _asset('lumit-0.2.0-linux-x64.flatpak', size: body.length),
        ]),
        download: (url, into,
                {required onProgress, required cancelled}) async =>
            into.writeAsBytesSync(body),
        launch: (file, _) async => launched.add(file),
        quit: () => quits.add(1),
      );
      await service.check();
      // The sandbox gets the bundle, never the tarball it cannot use.
      expect(service.release?.assetName, endsWith('.flatpak'));
      expect(service.delivery, UpdateDelivery.flatpakBundle);

      await service.downloadUpdate();
      await service.install();

      expect(launched, hasLength(1), reason: 'revealed, not run');
      expect(quits, isEmpty, reason: 'nothing of ours is being replaced');
      expect(service.installQuits, isFalse);
    });

    test('nothing is run before a download has finished', () async {
      final launched = <File>[];
      final service = _service(
        fetch: (_) async => _releaseJson(),
        launch: (file, _) async => launched.add(file),
      );
      await service.check();
      await service.install();
      expect(launched, isEmpty);
    });
  });

  group('choosing how to update', () {
    test('an installation Lumit cannot write to falls back to the installer',
        () {
      final release = UpdateRelease.parse(
        _releaseJson(),
        platform: 'windows',
        kind: InstallKind.folder,
        replaceable: false,
      );
      expect(release?.delivery, UpdateDelivery.installer);
      // **And it downloads the installer, not the archive**. This test
      // used to assert the opposite — the archive as the preferred asset, with
      // only the delivery changing — which is what shipped, and it cannot work:
      // `installer` delivery *runs the downloaded file*, and running a `.zip`
      // starts nothing. That is the v0.2 upgrade that downloaded, offered a
      // restart, did not restart, and came back on the old version.
      expect(release?.assetName, endsWith('setup.exe'));
    });
  });

  group('replacing Lumit in place', () {
    late Directory tmp;
    setUp(() => tmp = Directory.systemTemp.createTempSync('lumit-inplace'));
    tearDown(() {
      if (tmp.existsSync()) tmp.deleteSync(recursive: true);
    });

    /// A real little installation, and a service pointed at it whose "unpack"
    /// writes the new version out by hand.
    ({UpdateService service, InstallSite site, List<File> relaunched})
        installed({bool emptyArchive = false}) {
      final root = Directory('${tmp.path}/Lumit')..createSync(recursive: true);
      File('${root.path}/lumit_flutter').writeAsStringSync('0.1.0');
      final site = InstallSite(
        kind: InstallKind.folder,
        root: root,
        launcher: File('${root.path}/lumit_flutter'),
      );
      final relaunched = <File>[];
      final body = utf8.encode('a package');
      final service = _service(
        platform: 'linux',
        site: site,
        folder: () =>
            Directory('${tmp.path}/downloads')..createSync(recursive: true),
        fetch: (_) async => _releaseJson(assets: [
          _asset('lumit-0.2.0-linux-x64.tar.gz', size: body.length),
        ]),
        download: (url, into,
                {required onProgress, required cancelled}) async =>
            into.writeAsBytesSync(body),
        extract: (archive, into) async {
          if (emptyArchive) return;
          // What `tar` would leave: the tree inside its own wrapping folder.
          final tree = Directory('${into.path}/lumit-0.2.0-linux-x64')
            ..createSync(recursive: true);
          File('${tree.path}/lumit_flutter').writeAsStringSync('0.2.0');
        },
        relaunch: (launcher) async => relaunched.add(launcher),
      );
      return (service: service, site: site, relaunched: relaunched);
    }

    test('the new version replaces the old at the same path, and restarts',
        () async {
      final h = installed();
      await h.service.check();
      expect(h.service.delivery, UpdateDelivery.inPlace);
      await h.service.downloadUpdate();
      await h.service.install();

      expect(File('${h.site.root.path}/lumit_flutter').readAsStringSync(),
          '0.2.0');
      expect(h.relaunched, hasLength(1),
          reason: 'no installer — Lumit starts itself again');
      expect(h.site.previous.existsSync(), isTrue,
          reason: 'the old files are still open; the next launch sweeps them');
      expect(h.site.staging.existsSync(), isFalse);
    });

    test('an archive that unpacks to nothing leaves the old version standing',
        () async {
      final h = installed(emptyArchive: true);
      await h.service.check();
      await h.service.downloadUpdate();
      await h.service.install();

      expect(
          File('${h.site.root.path}/lumit_flutter').readAsStringSync(), '0.1.0',
          reason: 'the working Lumit is untouched');
      expect(h.service.stage, UpdateStage.failed);
      expect(h.relaunched, isEmpty);
      expect(h.site.unpacking.existsSync(), isFalse);
    });
  });

  group('the windows', () {
    late Directory scratch;
    setUp(() => scratch = Directory.systemTemp.createTempSync('lumit-update'));
    tearDown(() {
      if (scratch.existsSync()) scratch.deleteSync(recursive: true);
    });

    /// Everything the flow needs, with the answers a test can look at
    /// afterwards.
    ({Widget host, List<String> notices, List<int> saves}) harness(
      UpdateService service, {
      required bool dirty,
    }) {
      final notices = <String>[];
      final saves = <int>[];
      return (
        host: _Host(
          onPressed: (context) => pressUpdateRow(
            context,
            updates: service,
            notice: (message, {bool error = false}) => notices.add(message),
            projectIsDirty: () => dirty,
            saveProject: () async => saves.add(1),
          ),
        ),
        notices: notices,
        saves: saves,
      );
    }

    testWidgets('the whole sequence: offer, download, save and restart',
        (tester) async {
      final body = utf8.encode('installer');
      final launched = <File>[];
      final quits = <int>[];
      final service = _service(
        platform: 'windows',
        folder: () => scratch,
        fetch: (_) async => _releaseJson(assets: [
          _asset('lumit-0.2.0-windows-x64-setup.exe', size: body.length),
        ]),
        download: (url, into, {required onProgress, required cancelled}) async {
          onProgress(body.length, body.length);
          into.writeAsBytesSync(body);
        },
        launch: (file, _) async => launched.add(file),
        quit: () => quits.add(1),
      );
      await service.check();
      expect(service.stage, UpdateStage.available);

      final h = harness(service, dirty: true);
      await tester.pumpWidget(h.host);
      await tester.tap(find.byKey(const ValueKey('host-press')));
      await tester.pumpAndSettle();

      // The offer names the version and what it costs.
      expect(find.text('Update to Lumit 0.2.0?'), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('update-offer-yes')));
      await tester.pumpAndSettle();

      // Straight through the download to the restart question, which offers
      // to save because this project has unsaved work.
      expect(find.byKey(const ValueKey('update-save-restart')), findsOneWidget);
      expect(find.byKey(const ValueKey('update-restart-now')), findsOneWidget);
      await tester.tap(find.byKey(const ValueKey('update-save-restart')));
      await tester.pumpAndSettle();

      expect(h.saves, hasLength(1), reason: 'saved before quitting');
      expect(launched, hasLength(1));
      expect(quits, hasLength(1));
    });

    testWidgets(
        'a release with something to say says it before the restart question',
        (tester) async {
      final service = await _readyService(scratch, notes: _noticeBody);
      final h = harness(service, dirty: false);
      await tester.pumpWidget(h.host);
      await tester.tap(find.byKey(const ValueKey('host-press')));
      await tester.pumpAndSettle();

      // The notice, with the release's own words, and no restart question yet.
      expect(find.byKey(const ValueKey('update-notice')), findsOneWidget);
      expect(find.text('Before you update to Lumit 0.2.0'), findsOneWidget);
      expect(find.textContaining('Glow is drawn by a new renderer'),
          findsOneWidget);
      expect(find.byKey(const ValueKey('update-restart-now')), findsNothing);

      await tester.tap(find.byKey(const ValueKey('update-notice-continue')));
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('update-notice')), findsNothing);
      expect(find.byKey(const ValueKey('update-restart-now')), findsOneWidget);
    });
  });
}

/// Release notes with something to say before the update is applied, in the
/// shape the release pipeline puts at the top of a GitHub release: the section
/// first, the generated notes after it.
const String _noticeBody = '''
## Before you update

Glow is drawn by a new renderer, so a project that uses it
will not look the same as it did in 0.3.

- **Lens flare** loses its Streak count parameter.
- See [the notes](https://lumitlab.com/releases/0.4.0) for the rest.

## What's Changed

- Something else entirely
''';

/// A service already holding a verified download, for the windows that come
/// after one.
Future<UpdateService> _readyService(Directory scratch, {String? notes}) async {
  final body = utf8.encode('installer');
  final service = _service(
    platform: 'windows',
    folder: () => scratch,
    fetch: (_) async => _releaseJson(assets: [
      _asset('lumit-0.2.0-windows-x64-setup.exe', size: body.length),
    ], body: notes),
    download: (url, into, {required onProgress, required cancelled}) async =>
        into.writeAsBytesSync(body),
    launch: (file, _) async {},
    quit: () {},
  );
  await service.check();
  await service.downloadUpdate();
  return service;
}

/// A service with every seam stopped up. The default version is 0.1.0, which
/// is older than the 0.2.0 the sample release describes.
UpdateService _service({
  String? version = '0.1.0',
  String platform = 'windows',
  InstallSite? site,
  ReleaseFetcher? fetch,
  AssetDownloader? download,
  InstallerLauncher? launch,
  ArchiveExtractor? extract,
  Relauncher? relaunch,
  Quitter? quit,
  int Function()? now,
  Directory Function()? folder,
}) =>
    UpdateService(
      currentVersion: () => version,
      platform: platform,
      // Unknown by default, which means "use the installer": a test has to
      // ask for a replaceable installation before anything here will
      // contemplate swapping folders about, so the suite can never reach the
      // installation it is itself running from.
      site: site ?? _unknownSite,
      fetch: fetch ?? (_) async => _releaseJson(),
      download: download ??
          (url, into, {required onProgress, required cancelled}) async {},
      launch: launch ?? (file, _) async {},
      extract: extract ?? (archive, into) async {},
      relaunch: relaunch ?? (launcher) async {},
      quit: quit ?? () {},
      now: now,
      downloadFolder: folder ?? Directory.systemTemp.createTempSync,
    );

/// An installation Lumit cannot replace from inside — the safe default for
/// every test that is not about replacing one.
final InstallSite _unknownSite = InstallSite(
  kind: InstallKind.unknown,
  root: Directory('${Directory.systemTemp.path}/lumit-nowhere'),
  launcher: File('${Directory.systemTemp.path}/lumit-nowhere/lumit'),
);

/// The shape of GitHub's answer, with only the fields the updater reads.
Map<String, dynamic> _releaseJson({
  String tag = 'v0.2.0',
  bool draft = false,
  bool prerelease = false,
  List<Map<String, dynamic>>? assets,
  // The release notes. Left out, as an older API answer leaves it out.
  String? body,
}) =>
    {
      'tag_name': tag,
      'draft': draft,
      'prerelease': prerelease,
      'html_url': 'https://github.com/luminalmvm/lumit/releases/tag/$tag',
      if (body != null) 'body': body,
      // What a release actually carries: an installer and a package
      // per platform, plus the Flatpak bundle.
      'assets': assets ??
          [
            _asset('lumit-0.2.0-windows-x64-setup.exe'),
            _asset('lumit-0.2.0-windows-x64.zip'),
            _asset('lumit-0.2.0.dmg'),
            _asset('lumit-0.2.0-macos-arm64.zip'),
            _asset('lumit-0.2.0-linux-x64.tar.gz'),
            _asset('lumit-0.2.0-linux-x64.flatpak'),
          ],
    };

Map<String, dynamic> _asset(String name, {int size = 0, String? digest}) => {
      'name': name,
      'size': size,
      'browser_download_url':
          'https://github.com/luminalmvm/lumit/releases/download/v0.2.0/$name',
      if (digest != null) 'digest': digest,
    };

/// A window with an Overlay and one button, which is what the update windows
/// need to be shown into.
class _Host extends StatelessWidget {
  final Future<void> Function(BuildContext context) onPressed;
  const _Host({required this.onPressed});

  @override
  Widget build(BuildContext context) => Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Overlay(
            initialEntries: [
              OverlayEntry(
                builder: (context) => Center(
                  child: HouseButton(
                    key: const ValueKey('host-press'),
                    onPressed: () => onPressed(context),
                    child: const Text('Press'),
                  ),
                ),
              ),
            ],
          ),
        ),
      );
}
