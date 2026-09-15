// The addons service: the catalogue, the download and what the engine is told.
//
// Every seam this thing has with the outside world is injected, so the suite
// never reaches the network and never asks a real engine anything. What it does
// exercise is the sequence the Settings page draws: reading the catalogue,
// walking an install through its stages, refusing a file that is not the one
// that was published, stopping part way, and taking a pack away again.

import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart' as crypto;
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/addons.dart';
import 'package:lumit_flutter/state/download.dart';

void main() {
  late Directory scratch;

  setUp(() => scratch = Directory.systemTemp.createTempSync('lumit-addons'));
  tearDown(() {
    if (scratch.existsSync()) scratch.deleteSync(recursive: true);
  });

  /// The bytes one download carries, and the digest the catalogue publishes
  /// for them.
  final body = List<int>.generate(2048, (i) => i % 251);
  final digest = crypto.sha256.convert(body).toString();

  String indexJson({String? sha256, int? size}) => jsonEncode({
        'format': 1,
        'addons': [
          {
            'id': 'runtime',
            'kind': 'runtime',
            'name': 'ONNX Runtime',
            'version': '1.24.4',
            'licence': 'MIT',
            'size': 2048,
            'platforms': {
              'windows-x86_64': {
                'downloads': [
                  {
                    'url': 'https://example.invalid/onnxruntime.zip',
                    'sha256': sha256 ?? digest,
                    'size': size ?? 2048,
                    'unpack': 'zip',
                    'entries': {'runtimes/onnxruntime.dll': 'onnxruntime.dll'},
                  },
                ],
              },
            },
          },
          {
            'id': 'rife',
            'kind': 'model',
            'name': 'RIFE 4.9',
            'version': '1.0',
            'licence': 'MIT',
            'size': 2048,
            'model': {'task': 'synthesis', 'arch': 'rife', 'file': 'rife.onnx'},
            'platforms': {
              'any': {
                'downloads': [
                  {
                    'url': 'https://example.invalid/rife.onnx',
                    'sha256': sha256 ?? digest,
                    'size': size ?? 2048,
                    'unpack': 'file',
                    'dest': 'rife.onnx',
                  },
                ],
              },
            },
          },
          {
            'id': 'linux-only',
            'kind': 'model',
            'name': 'A pack for somebody else',
            'platforms': {
              'linux-x86_64': {
                'downloads': [
                  {
                    'url': 'https://example.invalid/other.onnx',
                    'sha256': digest,
                    'size': 2048,
                    'unpack': 'file',
                    'dest': 'other.onnx',
                  },
                ],
              },
            },
          },
        ],
      });

  /// A downloader that writes [body] in four pieces, asking to be stopped
  /// between each, and runs [between] after every one.
  AssetDownloader chunked({void Function()? between}) => (
        Uri url,
        File into, {
        required void Function(int received, int total) onProgress,
        required bool Function() cancelled,
      }) async {
        final sink = into.openSync(mode: FileMode.write);
        const pieces = 4;
        final step = body.length ~/ pieces;
        var sent = 0;
        try {
          for (var at = 0; at < body.length; at += step) {
            if (cancelled()) break;
            final piece = body.sublist(at, at + step);
            sink.writeFromSync(piece);
            sent += piece.length;
            onProgress(sent, body.length);
            between?.call();
          }
        } finally {
          sink.closeSync();
        }
      };

  /// The engine, as far as this file is concerned: what it has been told, and
  /// what it would answer.
  final installs = <({String manifest, List<String> files})>[];
  final removed = <String>[];
  var scan = <Addon>[];
  var refuse = false;
  var refuseRemove = false;

  setUp(() {
    installs.clear();
    removed.clear();
    scan = <Addon>[];
    refuse = false;
    refuseRemove = false;
  });

  AddonService serviceWith({
    TextFetcher? fetch,
    AssetDownloader? download,
  }) =>
      AddonService(
        list: () => scan,
        install: (manifest, files) async {
          if (refuse) throw StateError('the engine refused');
          installs.add((manifest: manifest, files: files));
        },
        remove: (id) {
          if (refuseRemove) throw StateError('the engine refused');
          removed.add(id);
          scan = [for (final a in scan) if (a.id != id) a];
        },
        runtime: () => const RuntimeStatus(state: RuntimeState.present),
        loadRuntime: () async =>
            const RuntimeStatus(state: RuntimeState.loaded, provider: 'CPU'),
        folder: () => scratch.path,
        fetch: fetch ?? (url) async => indexJson(),
        download: download ?? chunked(),
        platformKey: 'windows-x86_64',
      );

  /// Every stage the service passed through, in order.
  List<AddonStage> stagesOf(AddonService service) {
    final seen = <AddonStage>[];
    service.addListener(() {
      if (seen.isEmpty || seen.last != service.stage) seen.add(service.stage);
    });
    return seen;
  }

  group('the catalogue', () {
    test('is read into the entries the machine can install', () async {
      final service = serviceWith();
      await service.check();

      expect(service.stage, AddonStage.idle);
      expect(service.failure, isNull);
      expect([for (final e in service.entries) e.id], ['runtime', 'rife']);
      expect(service.runtimeOffered?.name, 'ONNX Runtime');
      expect([for (final e in service.available) e.id], ['rife'],
          reason: 'the runtime has its own section');
      final rife = service.entries.last;
      expect(rife.kind, AddonKind.model);
      expect(rife.task, 'synthesis');
      expect(rife.sizeBytes, 2048);
    });

    test('a block for this machine is preferred to the any block', () {
      final entries = parseCatalogue(
        jsonEncode({
          'format': 1,
          'addons': [
            {
              'id': 'both',
              'kind': 'model',
              'platforms': {
                'any': {
                  'downloads': [
                    {'url': 'any', 'sha256': digest, 'size': 1, 'dest': 'a'},
                  ],
                },
                'windows-x86_64': {
                  'downloads': [
                    {'url': 'win', 'sha256': digest, 'size': 1, 'dest': 'w'},
                  ],
                },
              },
            },
          ],
        }),
        platformKey: 'windows-x86_64',
      );

      expect(entries, hasLength(1));
      expect(downloadsFor(entries!.single.manifest!, 'windows-x86_64')!.single['url'],
          'win');
    });

    test('terms recorded beyond the licence come across with the entry', () {
      final addon = Addon.fromManifest(<String, dynamic>{
        'id': 'birefnet-lite',
        'kind': 'model',
        'licence': 'MIT',
        'notes': 'The weights are trained on DIS5K, whose terms are '
            'non-commercial.',
      });

      expect(addon!.notes, contains('non-commercial'),
          reason: "a pack whose training set carries terms of its own says so "
              'on its row, before anything is fetched');
    });

    test('something that is not a catalogue is said so, not thrown', () async {
      final service = serviceWith(fetch: (url) async => 'not json at all');
      await service.check();

      expect(service.stage, AddonStage.failed);
      expect(service.failure, AddonFailure.catalogue);
      expect(service.entries, isEmpty);
    });

    test('a network that is not there is said so', () async {
      final service =
          serviceWith(fetch: (url) async => throw const SocketException(''));
      await service.check();

      expect(service.stage, AddonStage.failed);
      expect(service.failure, AddonFailure.network);
    });
  });

  group('installing', () {
    test('walks down, checks, hands over, and says it is done', () async {
      final service = serviceWith();
      await service.check();
      final stages = stagesOf(service);

      await service.install('rife');

      expect(stages, [
        AddonStage.downloading,
        AddonStage.verifying,
        AddonStage.installing,
        AddonStage.done,
      ]);
      expect(service.failure, isNull);
      expect(installs, hasLength(1));
      final handed = jsonDecode(installs.single.manifest) as Map<String, dynamic>;
      expect(handed['id'], 'rife');
      expect(handed['model'], isA<Map<String, dynamic>>());
      expect(installs.single.files, hasLength(1));
      expect(File(installs.single.files.single).existsSync(), isFalse,
          reason: 'the temporary file goes once the engine has it');
      expect(service.working, isNull);
    });

    test('a file that is not the published one installs nothing', () async {
      final service = serviceWith(
          fetch: (url) async => indexJson(sha256: '0' * 64));
      await service.check();

      await service.install('rife');

      expect(service.stage, AddonStage.failed);
      expect(service.failure, AddonFailure.checksum);
      expect(installs, isEmpty);
      expect(
          Directory('${scratch.path}${Platform.pathSeparator}.downloads')
              .listSync(recursive: true)
              .whereType<File>(),
          isEmpty,
          reason: 'nothing is left behind');
    });

    test('a file that arrives short installs nothing', () async {
      final service = serviceWith(fetch: (url) async => indexJson(size: 9999));
      await service.check();

      await service.install('rife');

      expect(service.failure, AddonFailure.incomplete);
      expect(installs, isEmpty);
    });

    test('a second press while one is running is refused', () async {
      final service = serviceWith();
      await service.check();

      AddonFailure? refused;
      final first = service.install('rife');
      // The download is in flight, which is the only moment this can be asked.
      await service.install('runtime');
      refused = service.failure;
      await first;

      expect(refused, AddonFailure.busy);
      expect(installs, hasLength(1), reason: 'one at a time');
      expect(installs.single.files, hasLength(1));
    });

    test('an engine that refuses leaves the page saying so', () async {
      refuse = true;
      final service = serviceWith();
      await service.check();

      await service.install('rife');

      expect(service.stage, AddonStage.failed);
      expect(service.failure, AddonFailure.refused);
    });

    test('cancel stops between chunks and takes the part file with it',
        () async {
      late AddonService service;
      service = serviceWith(download: chunked(between: () => service.cancel()));
      await service.check();

      await service.install('rife');

      expect(service.stage, AddonStage.idle,
          reason: 'a cancelled download is not a failure');
      expect(service.failure, isNull);
      expect(service.working, isNull);
      expect(installs, isEmpty);
      expect(
          Directory('${scratch.path}${Platform.pathSeparator}.downloads')
              .listSync(recursive: true)
              .whereType<File>(),
          isEmpty);
    });
  });

  group('what is installed', () {
    test('remove tells the engine and reads the folder again', () {
      scan = const [
        Addon(id: 'rife', kind: AddonKind.model, name: 'RIFE 4.9'),
        Addon(id: 'runtime', kind: AddonKind.runtime, name: 'ONNX Runtime'),
      ];
      final service = serviceWith()..refresh();
      expect(service.packs, hasLength(1));
      expect(service.runtimeInstalled?.id, 'runtime');

      service.remove('rife');

      expect(removed, ['rife']);
      expect(service.packs, isEmpty, reason: 'the row goes with the re-read');
      expect(service.folder, scratch.path);
    });

    test('a pack already here at this version is not offered again', () async {
      scan = const [Addon(id: 'rife', kind: AddonKind.model, version: '1.0')];
      final service = serviceWith()..refresh();
      await service.check();

      expect(service.available, isEmpty);

      scan = const [Addon(id: 'rife', kind: AddonKind.model, version: '0.9')];
      service.refresh();

      expect([for (final e in service.available) e.id], ['rife'],
          reason: 'an older one is an update');

      scan = const [
        Addon(id: 'rife', kind: AddonKind.model, version: '1.0', broken: true)
      ];
      service.refresh();

      expect([for (final e in service.available) e.id], ['rife'],
          reason: 'a pack whose files have gone comes back as an offer, '
              'because installing it again is what mends it');
    });

    test('a removal the engine turns down is not called a failed install', () {
      refuseRemove = true;
      scan = const [Addon(id: 'runtime', kind: AddonKind.runtime)];
      final service = serviceWith()..refresh();

      service.remove('runtime');

      expect(service.failure, AddonFailure.removeRefused);
      expect(service.installed, hasLength(1), reason: 'the row stays');
    });

    test('what a run that died part way left is swept on the next read', () {
      final sep = Platform.pathSeparator;
      final stale = Directory('${scratch.path}$sep.downloads${sep}rife')
        ..createSync(recursive: true);
      File('${stale.path}${sep}0').writeAsBytesSync(body);

      serviceWith().refresh();

      expect(Directory('${scratch.path}$sep.downloads').existsSync(), isFalse,
          reason: 'nothing else ever looks in there, so a crash mid-fetch '
              'would leave the part file for good');
    });

    test('loading the runtime reports what it found', () async {
      final service = serviceWith();
      expect(service.runtime.state, RuntimeState.missing);

      await service.runtimeLoad();

      expect(service.runtime.state, RuntimeState.loaded);
      expect(service.runtime.provider, 'CPU');
    });
  });

  group('a pack from disk', () {
    test('is handed over as the files beside its description', () async {
      final folder = Directory('${scratch.path}${Platform.pathSeparator}pack')
        ..createSync();
      final sep = Platform.pathSeparator;
      File('${folder.path}${sep}rife.onnx').writeAsBytesSync(body);
      final manifest = File('${folder.path}${sep}addon.json')
        ..writeAsStringSync(jsonEncode({
          'format': 1,
          'id': 'rife',
          'kind': 'model',
          'name': 'RIFE 4.9',
          'model': {'task': 'synthesis', 'arch': 'rife', 'file': 'rife.onnx'},
          'platforms': {
            'any': {
              'downloads': [
                {
                  'url': 'https://example.invalid/rife.onnx',
                  'sha256': digest,
                  'size': 2048,
                  'unpack': 'file',
                  'dest': 'rife.onnx',
                },
              ],
            },
          },
        }));

      final service = serviceWith();
      await service.installFromFile(manifest.path);

      expect(service.stage, AddonStage.done);
      expect(installs, hasLength(1));
      expect(installs.single.files.single, '${folder.path}${sep}rife.onnx');
      final handed = jsonDecode(installs.single.manifest) as Map<String, dynamic>;
      final downloads =
          downloadsFor(handed, 'windows-x86_64')!;
      expect(downloads.single['dest'], 'rife.onnx');
      expect(downloads.single['sha256'], digest,
          reason: 'the digest is the file that is actually there');
      expect(downloads.single['size'], 2048);
    });

    test('a description whose files are not beside it installs nothing',
        () async {
      final manifest = File('${scratch.path}${Platform.pathSeparator}addon.json')
        ..writeAsStringSync(jsonEncode({
          'format': 1,
          'id': 'rife',
          'kind': 'model',
          'platforms': {
            'any': {
              'downloads': [
                {'url': 'x', 'sha256': digest, 'size': 1, 'dest': 'gone.onnx'},
              ],
            },
          },
        }));

      final service = serviceWith();
      await service.installFromFile(manifest.path);

      expect(service.stage, AddonStage.failed);
      expect(service.failure, AddonFailure.manifest);
      expect(installs, isEmpty);
    });
  });
}
