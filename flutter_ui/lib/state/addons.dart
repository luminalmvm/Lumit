// The addons the user can install, and the one download that installs them.
//
// # In plain terms
//
// A few things Lumit can do want a trained model and the library that runs one.
// Both are large files, they carry other people's licences, and most people
// never need either, so neither ships in the application: they are *addons*,
// listed on their own Settings page and fetched only when somebody presses the
// button (docs/impl/addons.md §5).
//
// This file is that button's working parts. It reads the catalogue, which is a
// single `index.json` in the `lumit-addons` repository. It streams each file the
// chosen addon names, checks every one against the size and digest the
// catalogue published, and then hands the engine the manifest text and the
// paths. The engine does the rest: it unpacks into a staging folder, writes the
// manifest beside the files and renames the folder into place, so a failure
// part way through leaves nothing behind.
//
// **Nothing here starts on its own.** No catalogue is read at launch, no file
// is fetched on a timer, and the only network traffic in the whole mechanism is
// a fetch of that one URL and a download of the files it names, both on a
// click. Every collaborator is injected, so the widget tests drive the whole
// sequence with no network and no engine.
//
// **No English is kept here.** A failure is one of [AddonFailure]; the page
// turns it into a sentence from the .arb. The service holds state, not copy.

import 'dart:convert';
import 'dart:ffi' show Abi;
import 'dart:io';

import 'package:crypto/crypto.dart' as crypto;
import 'package:flutter/foundation.dart';

import 'download.dart';

/// Where the official catalogue lives: one file on the `main` branch of the
/// second repository, read straight rather than through an API, so there is no
/// rate limit and nothing to authenticate.
const String addonCatalogueUrl =
    'https://raw.githubusercontent.com/luminalmvm/lumit-addons/main/index.json';

/// The two kinds of addon (docs/impl/addons.md §2). The runtime is the library
/// every model needs; a model pack is one model and its licence.
enum AddonKind { runtime, model }

/// What the engine says about the model runtime.
enum RuntimeState {
  /// Nothing is installed.
  missing,

  /// The files are there and nothing has asked for them yet.
  present,

  /// It loaded since Lumit started, and the provider and version are known.
  loaded,

  /// It is installed and would not load. The library's own words are in
  /// [RuntimeStatus.detail].
  failed,
}

/// How far an install has got. The order is the order a successful one walks
/// through them; [done] and [failed] are both endings.
enum AddonStage { idle, checking, downloading, verifying, installing, done, failed }

/// Why the last thing asked for did not happen. Turned into a sentence by the
/// page, never held here as one.
enum AddonFailure {
  /// The catalogue or a file could not be fetched.
  network,

  /// The catalogue came back as something this build cannot read.
  catalogue,

  /// A hand-picked `addon.json` could not be read, or the files it names are
  /// not beside it.
  manifest,

  /// The addon has nothing for this machine.
  unsupported,

  /// A file arrived at the wrong length.
  incomplete,

  /// A file arrived with the wrong digest.
  checksum,

  /// Something is already installing.
  busy,

  /// The engine refused, or there is nowhere to install to.
  refused,

  /// The engine would not delete the folder, which is what happens when the
  /// runtime it holds has already loaded.
  removeRefused,
}

/// What the engine knows about the model runtime, as the page draws it.
@immutable
class RuntimeStatus {
  final RuntimeState state;

  /// Which execution provider took the session: DirectML, CoreML or CPU. Empty
  /// until it has loaded.
  final String provider;

  /// The library's own version, empty until it has loaded.
  final String version;

  /// The library's own words when it would not load, untranslated, which is
  /// what the detail line is for.
  final String detail;

  const RuntimeStatus({
    this.state = RuntimeState.missing,
    this.provider = '',
    this.version = '',
    this.detail = '',
  });
}

/// One addon: a row in the catalogue, or one the engine has found installed.
@immutable
class Addon {
  final String id;
  final AddonKind kind;
  final String name;
  final String version;
  final String summary;

  /// The licence as an SPDX expression or a short name, shown as it is given.
  final String licence;
  final String licenceUrl;

  /// Terms the catalogue records beyond the licence itself, in the
  /// catalogue's own words: a training set whose terms differ from the code's,
  /// most often. Shown on the offer's row, before anything is fetched, because
  /// that is when the user is deciding. Empty for most packs.
  final String notes;

  /// What the whole addon weighs, for the size on the row.
  final int sizeBytes;

  /// What a model pack does: `depth`, `matte`, `segmentation` or `synthesis`.
  /// Empty for the runtime.
  final String task;

  /// Set by the engine when a file the manifest names is missing or the wrong
  /// size, so the row can say so instead of the effect failing later.
  final bool broken;

  /// The manifest this was read from, which is what the engine is handed on
  /// install. Null for an addon the engine listed: it already has the manifest
  /// on disk, and Dart never needs it again.
  final Map<String, dynamic>? manifest;

  const Addon({
    required this.id,
    required this.kind,
    this.name = '',
    this.version = '',
    this.summary = '',
    this.licence = '',
    this.licenceUrl = '',
    this.notes = '',
    this.sizeBytes = 0,
    this.task = '',
    this.broken = false,
    this.manifest,
  });

  /// Read one manifest object (docs/impl/addons.md §4). Null when it is not an
  /// object or has no id, which is as far as Dart judges a manifest: the engine
  /// is what refuses a format it does not know or a task it has no code for,
  /// and it does that with the text in front of it.
  static Addon? fromManifest(Object? json) {
    if (json is! Map<String, dynamic>) return null;
    final id = json['id'];
    if (id is! String || id.isEmpty) return null;
    final model = json['model'];
    return Addon(
      id: id,
      kind: json['kind'] == 'runtime' ? AddonKind.runtime : AddonKind.model,
      name: _text(json['name']),
      version: _text(json['version']),
      summary: _text(json['summary']),
      licence: _text(json['licence']),
      licenceUrl: _text(json['licence_url']),
      notes: _text(json['notes']),
      sizeBytes: json['size'] is num ? (json['size'] as num).toInt() : 0,
      task: model is Map ? _text(model['task']) : '',
      manifest: json,
    );
  }

  static String _text(Object? value) => value is String ? value : '';
}

/// The engine's scan of the addons folder.
typedef AddonLister = List<Addon> Function();

/// Handing the engine a manifest and the files it names, for it to unpack and
/// put in place. Throws when the engine refuses.
typedef AddonInstaller = Future<void> Function(
    String manifest, List<String> files);

/// Deleting an installed addon's folder. Throws when the engine refuses.
typedef AddonRemover = void Function(String id);

/// Reading the runtime's state, which is a value the engine already holds.
typedef RuntimeReader = RuntimeStatus Function();

/// Loading the runtime now and saying what happened.
typedef RuntimeLoader = Future<RuntimeStatus> Function();

/// Where addons live, as the engine has it, so the two sides cannot disagree
/// about the folder. Null on a machine with no home directory.
typedef AddonsFolder = String? Function();

/// The catalogue, what is installed, and one install at a time.
///
/// One instance while Lumit is running, on [LumitUiState], because the Settings
/// page and anything that links to it are two views of the same answer.
class AddonService extends ChangeNotifier {
  final AddonLister _list;
  final AddonInstaller _install;
  final AddonRemover _remove;
  final RuntimeReader _readRuntime;
  final RuntimeLoader _loadRuntime;
  final AddonsFolder _folderOf;
  final TextFetcher _fetch;
  final AssetDownloader _download;

  /// Which platform block of a manifest this machine reads
  /// (docs/impl/addons.md §4). Set outright in tests.
  final String platformKey;

  AddonService({
    required AddonLister list,
    required AddonInstaller install,
    required AddonRemover remove,
    required RuntimeReader runtime,
    required RuntimeLoader loadRuntime,
    required AddonsFolder folder,
    TextFetcher? fetch,
    AssetDownloader? download,
    String? platformKey,
  })  : _list = list,
        _install = install,
        _remove = remove,
        _readRuntime = runtime,
        _loadRuntime = loadRuntime,
        _folderOf = folder,
        _fetch = fetch ?? fetchText,
        _download = download ?? downloadAsset,
        platformKey = platformKey ?? defaultPlatformKey();

  List<Addon> _entries = const [];
  List<Addon> _installed = const [];
  RuntimeStatus _runtime = const RuntimeStatus();
  AddonStage _stage = AddonStage.idle;
  AddonFailure? _failure;
  double _fraction = 0;
  String? _working;
  String? _folder;
  bool _cancelRequested = false;

  /// What the catalogue offers, once it has been read. Empty until then.
  List<Addon> get entries => _entries;

  /// What the engine found in the addons folder, as of the last [refresh].
  List<Addon> get installed => _installed;

  /// The state of the model runtime, as of the last [refresh] or [runtimeLoad].
  RuntimeStatus get runtime => _runtime;

  AddonStage get stage => _stage;

  /// Why the last thing asked for did not happen, or null.
  AddonFailure? get failure => _failure;

  /// How far the download has got, 0 to 1. Zero at every other stage.
  double get fraction => _fraction;

  /// Which addon is being installed, or null.
  String? get working => _working;

  /// The addons folder, for the row that reveals it. Read on [refresh] rather
  /// than on demand, because the page must not cross the bridge to draw itself.
  String? get folder => _folder;

  /// Whether something is in flight, and so every button is unpressable.
  bool get busy =>
      _stage == AddonStage.checking ||
      _stage == AddonStage.downloading ||
      _stage == AddonStage.verifying ||
      _stage == AddonStage.installing;

  /// The installed runtime, or null when there is none.
  Addon? get runtimeInstalled => _firstOrNull(
      _installed.where((a) => a.kind == AddonKind.runtime));

  /// The catalogue's runtime entry, or null until the catalogue has been read.
  Addon? get runtimeOffered =>
      _firstOrNull(_entries.where((a) => a.kind == AddonKind.runtime));

  /// The model packs that are installed.
  List<Addon> get packs =>
      [for (final a in _installed) if (a.kind != AddonKind.runtime) a];

  /// The model packs the catalogue offers that are worth a button: not
  /// installed, installed at another version, which the row reads as Update,
  /// or installed with its files gone, because installing it again is the one
  /// thing that mends that.
  List<Addon> get available => [
        for (final e in _entries)
          if (e.kind != AddonKind.runtime && _worthOffering(e)) e
      ];

  bool _worthOffering(Addon offer) {
    final here = installedById(offer.id);
    return here == null || here.version != offer.version || here.broken;
  }

  /// The installed addon with this id, or null.
  Addon? installedById(String id) =>
      _firstOrNull(_installed.where((a) => a.id == id));

  /// Re-read what the engine knows: the scan, the runtime and the folder.
  /// Called when the page comes forward and after every install or removal.
  void refresh() {
    _installed = _list();
    _runtime = _readRuntime();
    _folder = _folderOf();
    _sweepDownloads();
    notifyListeners();
  }

  /// Throw away what a run that died part way left behind.
  ///
  /// The engine sweeps its own staging folder on every scan; `.downloads` is
  /// this side's and nothing else ever looks in it, so a crash or a power cut
  /// in the middle of a fetch would otherwise leave a few hundred megabytes in
  /// the user's data folder for good. Never while something is in flight: the
  /// file being written is in there.
  void _sweepDownloads() {
    final root = _folder;
    if (root == null || busy) return;
    try {
      final folder = Directory('$root${Platform.pathSeparator}.downloads');
      if (folder.existsSync()) folder.deleteSync(recursive: true);
    } catch (_) {
      // A file the system will not let go of is not worth a sentence about it;
      // the next time the page is opened it goes.
    }
  }

  /// Fetch the catalogue and hold what it offers this machine.
  ///
  /// Only on the button, and never fatal: no network, a moved file and a
  /// catalogue this build cannot read all land in [AddonStage.failed] with a
  /// reason the page says in one line.
  Future<void> check() async {
    if (busy) return;
    _stage = AddonStage.checking;
    _failure = null;
    notifyListeners();

    List<Addon>? found;
    try {
      found = parseCatalogue(await _fetch(Uri.parse(addonCatalogueUrl)),
          platformKey: platformKey);
    } catch (_) {
      _fail(AddonFailure.network);
      return;
    }
    if (found == null) {
      _fail(AddonFailure.catalogue);
      return;
    }
    _entries = found;
    _stage = AddonStage.idle;
    notifyListeners();
  }

  /// Install the catalogue entry called [id]: every file it names, verified,
  /// then handed to the engine with the manifest it came from.
  Future<void> install(String id) async {
    if (busy) {
      _refuse(AddonFailure.busy);
      return;
    }
    final entry = _firstOrNull(_entries.where((a) => a.id == id));
    final manifest = entry?.manifest;
    if (entry == null || manifest == null) {
      _fail(AddonFailure.refused);
      return;
    }
    final downloads = downloadsFor(manifest, platformKey);
    if (downloads == null || downloads.isEmpty) {
      _fail(AddonFailure.unsupported);
      return;
    }
    await _fetchAndInstall(entry, manifest, downloads);
  }

  /// Install a pack that is not in the catalogue, from an `addon.json` the user
  /// picked with the files it names beside it. This is how a machine with no
  /// network, or a pack Lumit does not list, gets one.
  ///
  /// The engine is handed a manifest whose downloads are those files, each with
  /// the digest and length it actually has, so the same copy and the same
  /// checks happen as for a catalogue install.
  Future<void> installFromFile(String path) async {
    if (busy) {
      _refuse(AddonFailure.busy);
      return;
    }
    final file = File(path);
    Addon? entry;
    try {
      entry = Addon.fromManifest(jsonDecode(file.readAsStringSync()));
    } catch (_) {
      entry = null;
    }
    final manifest = entry?.manifest;
    if (entry == null || manifest == null) {
      _fail(AddonFailure.manifest);
      return;
    }
    final names = packFileNames(manifest, platformKey);
    if (names.isEmpty) {
      _fail(AddonFailure.unsupported);
      return;
    }
    final beside = [
      for (final name in names)
        File('${file.parent.path}${Platform.pathSeparator}$name')
    ];
    if (beside.any((f) => !f.existsSync())) {
      _fail(AddonFailure.manifest);
      return;
    }

    _working = entry.id;
    _failure = null;
    _fraction = 0;
    _stage = AddonStage.installing;
    notifyListeners();

    final downloads = <Map<String, dynamic>>[];
    for (var i = 0; i < beside.length; i++) {
      final digest = await crypto.sha256.bind(beside[i].openRead()).first;
      downloads.add(<String, dynamic>{
        'url': beside[i].uri.toString(),
        'sha256': digest.toString(),
        'size': beside[i].lengthSync(),
        'unpack': 'file',
        'dest': names[i],
      });
    }
    final local = <String, dynamic>{
      ...manifest,
      'platforms': <String, dynamic>{
        'any': <String, dynamic>{'downloads': downloads},
      },
    };
    await _handOver(local, [for (final f in beside) f.path], discard: const []);
  }

  /// Delete an installed addon. The engine removes the folder; the row goes
  /// with the re-read that follows.
  void remove(String id) {
    if (busy) {
      _refuse(AddonFailure.busy);
      return;
    }
    try {
      _remove(id);
    } catch (_) {
      _fail(AddonFailure.removeRefused);
      return;
    }
    _failure = null;
    _stage = AddonStage.idle;
    refresh();
  }

  /// Abandon the download in flight. It stops between chunks and the partial
  /// file goes; the offer stands, because nothing about the addon has changed.
  void cancel() {
    if (_stage != AddonStage.downloading) return;
    _cancelRequested = true;
  }

  /// Load the runtime now and report what happened, which is the one way to
  /// find out whether an installed library actually works on this machine.
  Future<void> runtimeLoad() async {
    if (busy) return;
    _runtime = await _loadRuntime();
    notifyListeners();
  }

  /// The whole install: each file down, each file checked, then the engine.
  Future<void> _fetchAndInstall(
    Addon entry,
    Map<String, dynamic> manifest,
    List<Map<String, dynamic>> downloads,
  ) async {
    final root = _folderOf();
    if (root == null) {
      _fail(AddonFailure.refused);
      return;
    }
    final sep = Platform.pathSeparator;
    final folder = Directory('$root$sep.downloads$sep${entry.id}');

    _working = entry.id;
    _cancelRequested = false;
    _failure = null;
    _fraction = 0;
    _stage = AddonStage.downloading;
    notifyListeners();

    final files = <File>[];
    try {
      folder.createSync(recursive: true);
      final whole = downloads.fold<int>(0, (sum, d) => sum + _size(d));
      var behind = 0;
      for (var i = 0; i < downloads.length; i++) {
        final download = downloads[i];
        final url = download['url'];
        if (url is! String || url.isEmpty) {
          _discard(files);
          _fail(AddonFailure.catalogue);
          return;
        }
        final file = File('${folder.path}$sep$i');
        // A leftover from an abandoned attempt would otherwise be appended to.
        if (file.existsSync()) file.deleteSync();
        files.add(file);

        _stage = AddonStage.downloading;
        notifyListeners();
        await _download(
          Uri.parse(url),
          file,
          onProgress: (received, total) {
            final expected = whole > 0 ? whole : (total > 0 ? total : 0);
            final fraction =
                expected > 0 ? (behind + received) / expected : 0.0;
            final was = (_fraction * 100).round();
            _fraction = fraction.clamp(0.0, 1.0).toDouble();
            // Once per whole per cent, not once per chunk: every listener
            // rebuild costs far more than a chunk does, and no bar reads finer.
            if ((_fraction * 100).round() != was) notifyListeners();
          },
          cancelled: () => _cancelRequested,
        );
        if (_cancelRequested) {
          _discard(files);
          _idle();
          return;
        }

        _stage = AddonStage.verifying;
        notifyListeners();
        final problem = await verifySha256(file,
            sha256: download['sha256'] is String
                ? download['sha256'] as String
                : null,
            size: _size(download));
        if (problem != null) {
          _discard(files);
          _fail(problem == verifyChecksum
              ? AddonFailure.checksum
              : AddonFailure.incomplete);
          return;
        }
        behind += _size(download);
      }
    } catch (_) {
      _discard(files);
      _fail(AddonFailure.network);
      return;
    }

    await _handOver(manifest, [for (final f in files) f.path], discard: files);
  }

  /// Hand the engine the manifest and the paths, then put the page back
  /// together. The temporary files go whatever the answer is: the engine has
  /// copied what it needed by the time it returns.
  Future<void> _handOver(
    Map<String, dynamic> manifest,
    List<String> paths, {
    required List<File> discard,
  }) async {
    _stage = AddonStage.installing;
    _fraction = 1;
    notifyListeners();
    try {
      await _install(jsonEncode(manifest), paths);
    } catch (_) {
      _discard(discard);
      _fail(AddonFailure.refused);
      return;
    }
    _discard(discard);
    _stage = AddonStage.done;
    // A press turned down while this one ran has had its say by now, and the
    // line under the Check row belongs to what just happened.
    _failure = null;
    _working = null;
    refresh();
  }

  /// Back to resting, with nothing to report: what a cancelled download leaves.
  void _idle() {
    _stage = AddonStage.idle;
    _fraction = 0;
    _working = null;
    _failure = null;
    notifyListeners();
  }

  void _fail(AddonFailure why) {
    _stage = AddonStage.failed;
    _failure = why;
    _fraction = 0;
    _working = null;
    notifyListeners();
  }

  /// A press turned down without disturbing what is already running.
  void _refuse(AddonFailure why) {
    _failure = why;
    notifyListeners();
  }

  void _discard(List<File> files) {
    for (final file in files) {
      try {
        if (file.existsSync()) file.deleteSync();
      } catch (_) {
        // A file the system will not let go of is not worth failing an install
        // that has already succeeded; it is in a scratch folder.
      }
    }
  }

  static int _size(Map<String, dynamic> download) =>
      download['size'] is num ? (download['size'] as num).toInt() : 0;

  static T? _firstOrNull<T>(Iterable<T> of) {
    for (final item in of) {
      return item;
    }
    return null;
  }
}

/// Which platform block this machine reads: `windows-x86_64`,
/// `macos-aarch64`, `macos-x86_64` or `linux-x86_64`
/// (docs/impl/addons.md §4). `Abi` is where Dart says both halves at once.
String defaultPlatformKey() {
  final parts = Abi.current().toString().split('_');
  if (parts.length < 2) return parts.first;
  final architecture = switch (parts.last) {
    'x64' => 'x86_64',
    'arm64' => 'aarch64',
    final other => other,
  };
  return '${parts.first}-$architecture';
}

/// Read the catalogue: `{ "format": 1, "addons": [ <manifest>, ... ] }`.
///
/// Null when it is not that shape at all, which the page reports as a
/// catalogue this build cannot read. Entries with nothing for [platformKey] are
/// left out rather than listed with a button that could not work.
List<Addon>? parseCatalogue(String text, {required String platformKey}) {
  Object? json;
  try {
    json = jsonDecode(text);
  } catch (_) {
    return null;
  }
  if (json is! Map<String, dynamic>) return null;
  final addons = json['addons'];
  if (addons is! List) return null;
  final out = <Addon>[];
  for (final raw in addons) {
    final addon = Addon.fromManifest(raw);
    if (addon == null) continue;
    final downloads = downloadsFor(addon.manifest!, platformKey);
    if (downloads == null || downloads.isEmpty) continue;
    out.add(addon);
  }
  return out;
}

/// The downloads a manifest lists for [platformKey], falling back to its `any`
/// block. Null when it has neither.
List<Map<String, dynamic>>? downloadsFor(
    Map<String, dynamic> manifest, String platformKey) {
  final platforms = manifest['platforms'];
  if (platforms is! Map) return null;
  final block = platforms[platformKey] ?? platforms['any'];
  if (block is! Map) return null;
  final downloads = block['downloads'];
  if (downloads is! List) return null;
  return [
    for (final d in downloads)
      if (d is Map<String, dynamic>) d,
  ];
}

/// The files a manifest puts in the addon's folder: every download's `dest`,
/// and every name a zip download takes out of its archive.
List<String> packFileNames(Map<String, dynamic> manifest, String platformKey) {
  final out = <String>[];
  for (final download in downloadsFor(manifest, platformKey) ?? const []) {
    final dest = download['dest'];
    if (dest is String && dest.isNotEmpty) out.add(dest);
    final entries = download['entries'];
    if (entries is Map) {
      for (final name in entries.values) {
        if (name is String && name.isNotEmpty) out.add(name);
      }
    }
  }
  return out;
}
