// Looking for a newer Lumit, and fetching it (K-296).
//
// # In plain terms
//
// Releases are published as GitHub Releases on a `v*` tag, and every one of
// them carries the finished installers as attachments: a `setup.exe` for
// Windows, a `.dmg` for macOS, a `.tar.gz` and a `.flatpak` for Linux. So
// "is there a newer Lumit?" is one small web request — GitHub will say what the
// latest release is called and what is attached to it — and "get it" is an
// ordinary download of the attachment that suits this machine.
//
// This file is that, and nothing else: it knows the versions, the download and
// the file on disk. It draws no windows and it never quits the application; the
// shell does both (`shell/update_dialog_frb.dart`), because *when* to ask the
// user something is a question about the interface, not about updating.
//
// **Full installers, never patches (K-296).** The download is the whole
// installer every time. A patch system means publishing a patch per pair of
// versions, a tool to apply them, and a fallback for when the pair is missing —
// three new things that can go wrong to save bandwidth GitHub gives us for
// nothing. A few hundred megabytes, once in a while, on a deliberate click.
//
// **Nothing is downloaded behind the user's back.** With automatic updates on,
// Lumit *looks* on launch, at most once a day, and says what it found in the
// menu. Fetching the installer always waits for a click — this is a video
// application, and someone who is mid-edit on a hotel connection should not
// discover Lumit spending their data.

import 'dart:convert';
import 'dart:io';

import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';

import 'install_site.dart';
import 'package:lumit_flutter/l10n/strings.dart';

/// The repository releases are published from (K-279: the website reads the
/// same one).
const String updatesRepository = 'luminalmvm/lumit';

/// Where the newest release is described.
Uri get latestReleaseUrl => Uri.parse(
    'https://api.github.com/repos/$updatesRepository/releases/latest');

/// How long a check is good for. A day: long enough that launching Lumit six
/// times in a morning asks GitHub once, short enough that a release is noticed
/// the day after it lands.
const Duration updateCheckInterval = Duration(hours: 24);

/// Where the state machine has got to.
///
/// The order is the order a successful update walks through them; [upToDate]
/// and [failed] are both endings that go back to being able to check again.
enum UpdateStage {
  /// Nothing has been asked yet this session.
  idle,

  /// A check is in flight.
  checking,

  /// Checked, and this is the newest Lumit there is.
  upToDate,

  /// A newer release exists and has an installer for this machine.
  available,

  /// That installer is coming down now.
  downloading,

  /// It is on disk, verified, and waiting for a restart.
  ready,

  /// The check or the download did not finish. Recoverable: the menu goes back
  /// to offering a check.
  failed,
}

/// How a downloaded release gets applied (K-297).
enum UpdateDelivery {
  /// Unpacked beside the installation and swapped in on restart — no installer,
  /// no elevation, the way Chrome and VS Code do it. What a per-user
  /// installation gets.
  inPlace,

  /// Handed to the installer that built it: an older installation in
  /// `Program Files`, a macOS disk image, anywhere Lumit cannot write to its
  /// own files.
  installer,

  /// Handed to Flatpak, which owns updating inside its own sandbox.
  flatpakBundle,
}

/// A release that is newer than this build, and the file to fetch for it.
@immutable
class UpdateRelease {
  /// The version without its leading `v` — `0.2.0`.
  final String version;

  /// The tag as GitHub has it — `v0.2.0`.
  final String tag;

  /// The release's page, for the notes.
  final String pageUrl;

  /// The attachment that suits this machine.
  final String assetName;
  final Uri assetUrl;

  /// What GitHub says the attachment weighs. Checked after the download, so a
  /// truncated file is caught before anything is run.
  final int assetBytes;

  /// What will be done with the attachment once it is here.
  final UpdateDelivery delivery;

  /// The attachment's SHA-256 as `sha256:…`, when the API gives one. Newer
  /// GitHub responses carry a `digest` field; older ones do not, and a release
  /// without it is verified by size alone.
  final String? sha256;

  const UpdateRelease({
    required this.version,
    required this.tag,
    required this.pageUrl,
    required this.assetName,
    required this.assetUrl,
    required this.assetBytes,
    required this.delivery,
    this.sha256,
  });

  /// How big the download is, for a sentence a human reads. Whole MB: the
  /// difference between 412 and 412.4 MB is not a decision anybody makes.
  String get sizeLabel => '${(assetBytes / (1 << 20)).round()} MB';

  /// Read a release out of the GitHub API's answer, picking the attachment for
  /// [platform]. Null when the release has nothing this machine can install —
  /// a release whose Windows job failed, say, seen from Windows. Null rather
  /// than an error: there is genuinely no update *for this machine*, and
  /// offering one that cannot be installed is worse than offering none.
  static UpdateRelease? parse(
    Map<String, dynamic> json, {
    required String platform,
    InstallKind kind = InstallKind.unknown,
    bool replaceable = false,
  }) {
    final tag = json['tag_name'];
    if (tag is! String || tag.isEmpty) return null;
    // Draft releases are not published and pre-releases are not what `latest`
    // returns, but both are cheap to refuse and expensive to ship by accident.
    if (json['draft'] == true || json['prerelease'] == true) return null;

    final assets = json['assets'];
    if (assets is! List) return null;

    Map<String, dynamic>? chosen;
    for (final suffix in assetSuffixesFor(platform, kind: kind)) {
      for (final raw in assets) {
        if (raw is! Map) continue;
        final asset = raw.cast<String, dynamic>();
        final name = asset['name'];
        if (name is String && name.toLowerCase().endsWith(suffix)) {
          chosen = asset;
          break;
        }
      }
      if (chosen != null) break;
    }
    if (chosen == null) return null;

    final url = chosen['browser_download_url'];
    if (url is! String) return null;
    final size = chosen['size'];

    final name = chosen['name'] as String;
    return UpdateRelease(
      version: versionFromTag(tag),
      tag: tag,
      pageUrl: json['html_url'] is String ? json['html_url'] as String : '',
      assetName: name,
      assetUrl: Uri.parse(url),
      assetBytes: size is int ? size : 0,
      delivery: deliveryFor(name, kind: kind, replaceable: replaceable),
      sha256: chosen['digest'] is String ? chosen['digest'] as String : null,
    );
  }
}

/// Which attachments suit this machine, best first (K-297).
///
/// A per-user installation prefers the *package* — the plain archive of the
/// application's own files — because that can be swapped in without an
/// installer or an administrator. The installer stays second on the list, for
/// an older installation sitting somewhere only an installer can write to.
///
/// A Flatpak is offered the Flatpak bundle and nothing else: the sandbox cannot
/// be updated from inside, so the other attachments would be useless there.
List<String> assetSuffixesFor(String platform,
    {InstallKind kind = InstallKind.unknown}) {
  if (kind == InstallKind.flatpak) return const ['.flatpak'];
  switch (platform) {
    case 'windows':
      return kind == InstallKind.folder
          ? const ['windows-x64.zip', '.exe']
          : const ['.exe'];
    case 'macos':
      return kind == InstallKind.bundle
          ? const ['macos-arm64.zip', 'macos-x64.zip', 'macos.zip', '.dmg']
          : const ['.dmg'];
    default:
      return const ['linux-x64.tar.gz', '.tar.gz', '.flatpak'];
  }
}

/// What will happen to an attachment once it has been fetched.
///
/// The archive is only applied in place when this installation can genuinely
/// be written to — otherwise the archive is no use and the file we have is an
/// installer, whatever its extension.
UpdateDelivery deliveryFor(
  String assetName, {
  required InstallKind kind,
  required bool replaceable,
}) {
  if (kind == InstallKind.flatpak) return UpdateDelivery.flatpakBundle;
  final name = assetName.toLowerCase();
  final isArchive = name.endsWith('.zip') || name.endsWith('.tar.gz');
  return isArchive && replaceable
      ? UpdateDelivery.inPlace
      : UpdateDelivery.installer;
}

/// `v0.2.0` → `0.2.0`. Anything else is handed back unchanged, so an oddly
/// named tag is compared rather than silently treated as version zero.
String versionFromTag(String tag) =>
    tag.startsWith('v') ? tag.substring(1) : tag;

/// The version out of a boot-log line — `lumit-bridge 0.1.0` → `0.1.0`.
///
/// The boot log is where this build's version already lives (K-008), so there
/// is no second source of truth to keep in step. Null when the line is not
/// shaped like that, which is what a test harness with no engine sees.
String? versionFromBootLine(String line) {
  final match = RegExp(r'(\d+\.\d+\.\d+[^\s]*)').firstMatch(line);
  return match?.group(1);
}

/// Compare two versions: negative when [a] is older, zero when they are the
/// same release, positive when [a] is newer.
///
/// Semantic versioning, as much of it as tags actually use: the three numbers
/// compared as numbers, then the pre-release suffix, where *having* one makes a
/// version older than the same numbers without one (`0.2.0-rc.1` < `0.2.0`).
/// Build metadata after `+` is not part of the ordering, so it is dropped.
int compareVersions(String a, String b) {
  (List<int>, String) split(String raw) {
    final noBuild = raw.split('+').first.trim();
    final dash = noBuild.indexOf('-');
    final core = dash < 0 ? noBuild : noBuild.substring(0, dash);
    final pre = dash < 0 ? '' : noBuild.substring(dash + 1);
    final numbers = [
      for (final part in core.split('.')) int.tryParse(part) ?? 0,
    ];
    // Missing places are zero: `1.2` is `1.2.0`.
    while (numbers.length < 3) {
      numbers.add(0);
    }
    return (numbers, pre);
  }

  final (leftNumbers, leftPre) = split(a);
  final (rightNumbers, rightPre) = split(b);
  for (var i = 0; i < 3; i++) {
    final diff = leftNumbers[i].compareTo(rightNumbers[i]);
    if (diff != 0) return diff;
  }
  if (leftPre == rightPre) return 0;
  // A release beats its own pre-releases.
  if (leftPre.isEmpty) return 1;
  if (rightPre.isEmpty) return -1;
  return leftPre.compareTo(rightPre);
}

/// Asking GitHub what the latest release is. Injected so a test can answer
/// without a network.
typedef ReleaseFetcher = Future<Map<String, dynamic>> Function(Uri url);

/// Fetching the installer. [onProgress] is called with bytes received and the
/// total expected; [cancelled] is asked as it goes and a true answer abandons
/// the download. Injected for the same reason.
typedef AssetDownloader = Future<void> Function(
  Uri url,
  File into, {
  required void Function(int received, int total) onProgress,
  required bool Function() cancelled,
});

/// Handing the downloaded file to the system. Injected so a test never runs an
/// installer, which is the one thing in this file that cannot be undone.
typedef InstallerLauncher = Future<void> Function(File file, String platform);

/// Unpacking a downloaded archive into a folder. The platform's own tool does
/// this in the shipped application (see `_extractArchive`); a test hands over a
/// tree it made itself.
typedef ArchiveExtractor = Future<void> Function(File archive, Directory into);

/// Starting the freshly swapped-in Lumit, once the old one is about to go.
typedef Relauncher = Future<void> Function(File launcher);

/// Ending the process so the installer can replace the files underneath it.
typedef Quitter = void Function();

/// The whole update state machine, shared by the Help menu and Settings.
///
/// One instance for the session, on [LumitUiState], so the menu and the
/// Settings page cannot disagree about what is happening — they are two views
/// of this object, and both redraw from its notifications.
class UpdateService extends ChangeNotifier {
  /// What this build is, asked lazily. A function rather than a string because
  /// the answer comes over the bridge, and a service constructed in a widget
  /// test must not call the engine merely by existing.
  final String? Function() currentVersion;

  /// Which platform's attachment to look for. `Platform.operatingSystem` in
  /// the shipped application; set outright in tests.
  final String platform;

  /// Where this copy of Lumit lives and whether it may replace itself (K-297).
  final InstallSite site;

  final ReleaseFetcher _fetch;
  final AssetDownloader _download;
  final InstallerLauncher _launch;
  final ArchiveExtractor _extract;
  final Relauncher _relaunch;
  final Quitter _quit;

  /// Now, in milliseconds since the epoch. Injected so the once-a-day rule can
  /// be tested without waiting a day.
  final int Function() _now;

  /// Where the installer is put. The system temporary folder in the shipped
  /// application: it is a file the operating system may clean up, and once the
  /// update is installed there is no reason to keep it.
  final Directory Function() _downloadFolder;

  UpdateService({
    required this.currentVersion,
    String? platform,
    InstallSite? site,
    ReleaseFetcher? fetch,
    AssetDownloader? download,
    InstallerLauncher? launch,
    ArchiveExtractor? extract,
    Relauncher? relaunch,
    Quitter? quit,
    int Function()? now,
    Directory Function()? downloadFolder,
  })  : platform = platform ?? Platform.operatingSystem,
        site = site ?? InstallSite.detect(),
        _fetch = fetch ?? _fetchReleaseJson,
        _download = download ?? _downloadAsset,
        _launch = launch ?? _launchInstaller,
        _extract = extract ?? _extractArchive,
        _relaunch = relaunch ?? _relaunchLumit,
        _quit = quit ?? _exitProcess,
        _now = now ?? _epochMillis,
        _downloadFolder = downloadFolder ?? _defaultDownloadFolder;

  UpdateStage _stage = UpdateStage.idle;
  UpdateRelease? _release;
  File? _downloaded;
  double _progress = 0;
  String? _failure;
  bool _cancelRequested = false;

  UpdateStage get stage => _stage;

  /// The release that is waiting, once one has been found.
  UpdateRelease? get release => _release;

  /// How far the download has got, 0 to 1. Zero at every other stage.
  double get progress => _progress;

  /// Why the last attempt did not finish, in a sentence fit for the status
  /// line. Null unless [stage] is [UpdateStage.failed].
  String? get failure => _failure;

  /// The installer on disk, once [UpdateStage.ready].
  File? get downloadedInstaller => _downloaded;

  /// Whether something is in flight, and so neither the menu row nor the
  /// Settings button should be pressable.
  bool get busy =>
      _stage == UpdateStage.checking || _stage == UpdateStage.downloading;

  /// What the Help menu's row reads (K-296).
  ///
  /// The wording is the state: an update that has been found says so with its
  /// version in the row itself, which is the one place somebody is already
  /// looking when they wonder whether to update. "Check for updates" is both
  /// the resting state and where a finished check with nothing to report goes
  /// back to — a row that stayed on "You are up to date" would be a stale
  /// claim by the next morning.
  String get menuLabel => switch (_stage) {
        UpdateStage.checking => l10n.updateChecking,
        UpdateStage.available =>
          l10n.updateClickToUpdate(_release?.version ?? ''),
        UpdateStage.downloading =>
          l10n.updateDownloadingPercent('${(_progress * 100).round()}'),
        UpdateStage.ready => l10n.updateRestartToFinish,
        _ => l10n.updateCheckFor,
      };

  /// Whether enough time has passed to look again.
  bool dueForCheck(int lastCheckedMillis) =>
      _now() - lastCheckedMillis >= updateCheckInterval.inMilliseconds;

  /// Ask GitHub what the newest release is.
  ///
  /// Never throws: a machine with no network, a rate-limited API and a
  /// malformed answer all land in [UpdateStage.failed] with a sentence, because
  /// none of them is a reason for an editor to stop working.
  Future<void> check() async {
    if (busy) return;
    _stage = UpdateStage.checking;
    _failure = null;
    notifyListeners();

    try {
      final current = currentVersion();
      if (current == null) {
        // No version to compare against — every release would look newer.
        _fail(l10n.updateUnknownVersion);
        return;
      }
      final json = await _fetch(latestReleaseUrl);
      final found = UpdateRelease.parse(
        json,
        platform: platform,
        kind: site.kind,
        replaceable: site.replaceable,
      );
      if (found == null || compareVersions(found.version, current) <= 0) {
        _stage = UpdateStage.upToDate;
        _release = null;
      } else {
        _stage = UpdateStage.available;
        _release = found;
      }
    } catch (_) {
      _fail(l10n.updateCheckFailed);
      return;
    }
    notifyListeners();
  }

  /// Fetch the waiting release's installer and verify it.
  ///
  /// Ends at [UpdateStage.ready] with [downloadedInstaller] set, or back at
  /// [UpdateStage.available] when cancelled — a cancelled download leaves the
  /// offer standing, since nothing about the release has changed.
  Future<void> downloadUpdate() async {
    final release = _release;
    if (release == null || busy) return;
    _cancelRequested = false;
    _progress = 0;
    _stage = UpdateStage.downloading;
    _failure = null;
    notifyListeners();

    File? file;
    try {
      final folder = _downloadFolder();
      folder.createSync(recursive: true);
      file =
          File('${folder.path}${Platform.pathSeparator}${release.assetName}');
      // A leftover from an abandoned attempt would otherwise be appended to or
      // mistaken for a finished download.
      if (file.existsSync()) file.deleteSync();

      await _download(
        release.assetUrl,
        file,
        onProgress: (received, total) {
          // `contentLength` is -1 when the server does not say, in which case
          // the size the release published is the best answer there is.
          final expected = total > 0 ? total : release.assetBytes;
          final fraction = expected > 0 ? received / expected : 0.0;
          final was = (_progress * 100).round();
          _progress = fraction.clamp(0.0, 1.0).toDouble();
          // Once per whole per cent, not once per HTTP chunk: every listener
          // rebuild (the menu bar among them) is far dearer than a download
          // chunk, and no progress bar reads finer than a per cent anyway.
          if ((_progress * 100).round() != was) notifyListeners();
        },
        cancelled: () => _cancelRequested,
      );

      if (_cancelRequested) {
        _discard(file);
        _stage = UpdateStage.available;
        _progress = 0;
        notifyListeners();
        return;
      }

      final problem = await _verify(file, release);
      if (problem != null) {
        _discard(file);
        _fail(problem);
        return;
      }

      _downloaded = file;
      _progress = 1;
      _stage = UpdateStage.ready;
    } catch (_) {
      if (file != null) _discard(file);
      _fail(_cancelRequested
          ? l10n.updateDownloadCancelled
          : l10n.updateDownloadFailed);
      return;
    }
    notifyListeners();
  }

  /// Abandon a download in progress. Safe at any other stage, where it does
  /// nothing.
  void cancelDownload() {
    if (_stage != UpdateStage.downloading) return;
    _cancelRequested = true;
  }

  /// Check the file against what the release said it would be.
  ///
  /// Returns null when it is sound, or the sentence to show when it is not.
  /// This is the gate before anything is executed: an installer is the most
  /// dangerous file Lumit ever touches, so it is run only when its length and
  /// — where GitHub publishes one — its digest are exactly what the release
  /// described.
  ///
  /// The length is read synchronously on purpose: it keeps the whole sequence
  /// — offer, download, verify, restart — free of real asynchronous IO except
  /// where a digest genuinely needs streaming, which is what lets a widget test
  /// drive the windows from end to end (`flutter_test` does not run real IO
  /// outside `runAsync`).
  Future<String?> _verify(File file, UpdateRelease release) async {
    final length = file.lengthSync();
    if (release.assetBytes > 0 && length != release.assetBytes) {
      return l10n.updateIncomplete;
    }
    final expected = release.sha256;
    if (expected == null) return null;
    final wanted = expected.startsWith('sha256:')
        ? expected.substring('sha256:'.length)
        : expected;
    final digest = await sha256.bind(file.openRead()).first;
    if (digest.toString().toLowerCase() != wanted.toLowerCase()) {
      return l10n.updateChecksumMismatch;
    }
    return null;
  }

  /// What will happen when the waiting update is applied. [UpdateDelivery
  /// .installer] until a release has been found, since that is the cautious
  /// answer.
  UpdateDelivery get delivery => _release?.delivery ?? UpdateDelivery.installer;

  /// Whether finishing the update means leaving the application.
  ///
  /// True for the two that replace what is running — the in-place swap and the
  /// installer. False for a Flatpak, where Lumit only hands the file over and
  /// carries on.
  bool get installQuits => delivery != UpdateDelivery.flatpakBundle;

  /// Apply the update that is waiting, whichever of the three ways this
  /// installation calls for (K-297).
  ///
  /// In place: unpack beside the installation, swap the two folders, start the
  /// new Lumit and leave. By installer: start it and leave, because it needs to
  /// write where we cannot. Flatpak: reveal the bundle and stay open, because
  /// the sandbox is not ours to rewrite.
  Future<void> install() async {
    final file = _downloaded;
    if (file == null || _stage != UpdateStage.ready) return;

    if (delivery == UpdateDelivery.inPlace) {
      await _applyInPlace(file);
      return;
    }
    try {
      await _launch(file, platform);
    } catch (_) {
      _fail(l10n.updateInstallerFailed);
      return;
    }
    if (delivery == UpdateDelivery.installer) _quit();
  }

  /// The installer-free path: unpack, stage, swap, restart (K-297).
  ///
  /// Every step before the swap is undoable by deleting a folder, and the swap
  /// itself puts the old version back if it cannot finish — so a failure here
  /// leaves the working Lumit exactly where it was, and says so.
  Future<void> _applyInPlace(File archive) async {
    try {
      // Anything left from a previous attempt would be mistaken for this one.
      _sweep(site.unpacking);
      _sweep(site.staging);
      site.unpacking.createSync(recursive: true);

      await _extract(archive, site.unpacking);
      final tree = unwrapSingleFolder(site.unpacking);
      // An archive that unpacked to nothing is not something to swap a working
      // application for. The checksum already proved the *file*; this proves
      // there is an application inside it.
      if (tree.listSync().isEmpty) {
        _sweep(site.unpacking);
        _fail(l10n.updateEmpty);
        return;
      }
      // Onto the final name, on the same filesystem, so the swap that follows
      // is a rename and not a copy.
      tree.renameSync(site.staging.path);
      _sweep(site.unpacking);
      markStagedUpdateReady(site);

      swapInStagedUpdate(site);
    } catch (_) {
      _sweep(site.unpacking);
      _sweep(site.staging);
      _fail(l10n.updateSwapFailed);
      return;
    }

    try {
      await _relaunch(site.launcher);
    } catch (_) {
      // The files are already the new version, so there is nothing to undo —
      // starting Lumit again by hand gets the update either way.
      _fail(l10n.updateRestartFailed);
      return;
    }
    _quit();
  }

  void _sweep(Directory dir) {
    try {
      if (dir.existsSync()) dir.deleteSync(recursive: true);
    } catch (_) {
      // Litter beside the installation, not a reason to stop.
    }
  }

  void _discard(File file) {
    try {
      if (file.existsSync()) file.deleteSync();
    } catch (_) {
      // A file we cannot delete is litter in a temporary folder, not a fault
      // worth showing anybody.
    }
  }

  void _fail(String message) {
    _stage = UpdateStage.failed;
    _failure = message;
    _progress = 0;
    notifyListeners();
  }
}

// --- The real implementations ---------------------------------------------

int _epochMillis() => DateTime.now().millisecondsSinceEpoch;

void _exitProcess() => exit(0);

Directory _defaultDownloadFolder() =>
    Directory('${Directory.systemTemp.path}${Platform.pathSeparator}'
        'lumit-update');

/// GitHub wants a user agent and answers JSON. Nothing is authenticated: the
/// releases of a public repository are public, and asking anonymously means
/// Lumit never holds a token.
Future<Map<String, dynamic>> _fetchReleaseJson(Uri url) async {
  final client = HttpClient()..connectionTimeout = const Duration(seconds: 10);
  try {
    final request = await client.getUrl(url);
    request.headers.set(HttpHeaders.userAgentHeader, 'Lumit');
    request.headers
        .set(HttpHeaders.acceptHeader, 'application/vnd.github+json');
    final response = await request.close();
    if (response.statusCode != 200) {
      // Drained rather than dropped: an undrained response holds the socket.
      await response.drain<void>();
      throw HttpException(l10n.updateServerAnswered('${response.statusCode}'),
          uri: url);
    }
    final body = await response.transform(utf8.decoder).join();
    final json = jsonDecode(body);
    if (json is! Map<String, dynamic>) {
      throw FormatException(l10n.updateBadReleaseData);
    }
    return json;
  } finally {
    client.close(force: true);
  }
}

/// Stream the attachment to disk. GitHub redirects asset URLs at its CDN, which
/// `HttpClient` follows; the file is written as it arrives rather than held in
/// memory, because these are hundreds of megabytes.
Future<void> _downloadAsset(
  Uri url,
  File into, {
  required void Function(int received, int total) onProgress,
  required bool Function() cancelled,
}) async {
  final client = HttpClient()..connectionTimeout = const Duration(seconds: 10);
  IOSink? sink;
  try {
    final request = await client.getUrl(url);
    request.headers.set(HttpHeaders.userAgentHeader, 'Lumit');
    final response = await request.close();
    if (response.statusCode != 200) {
      await response.drain<void>();
      throw HttpException(l10n.updateDownloadAnswered('${response.statusCode}'),
          uri: url);
    }
    final total = response.contentLength;
    var received = 0;
    sink = into.openWrite();
    await for (final chunk in response) {
      if (cancelled()) break;
      sink.add(chunk);
      received += chunk.length;
      onProgress(received, total);
    }
  } finally {
    await sink?.flush();
    await sink?.close();
    client.close(force: true);
  }
}

/// Start the installer, detached, so it outlives the process that started it —
/// which it has to, since that process is about to end.
///
/// Only reached where the update could *not* be applied in place (K-297): an
/// installation somewhere Lumit cannot write, or a macOS disk image.
Future<void> _launchInstaller(File file, String platform) async {
  switch (platform) {
    case 'windows':
      // Inno Setup's own switches (packaging/windows/lumit.iss). `/SILENT`
      // shows progress and no questions — the questions were all answered when
      // Lumit was first installed, and asking them again to apply an update the
      // user has already agreed to would be ceremony. `/CLOSEAPPLICATIONS`
      // lets it deal with anything of ours still holding a file, and
      // `/NORESTART` means it never reboots the machine on its own.
      await Process.start(
        file.path,
        const ['/SILENT', '/CLOSEAPPLICATIONS', '/NORESTART'],
        mode: ProcessStartMode.detached,
      );
    case 'macos':
      await Process.start('open', [file.path], mode: ProcessStartMode.detached);
    default:
      // A Flatpak bundle: revealed, never run. `flatpak install` is the user's
      // to run, and a sandboxed Lumit has no business reaching the host to do
      // it for them (K-297).
      await Process.start('xdg-open', [file.parent.path],
          mode: ProcessStartMode.detached);
  }
}

/// Unpack a downloaded archive with the tool the platform already has.
///
/// Not a Dart zip library, deliberately: a macOS `.app` and a Linux bundle carry
/// symbolic links and executable permissions, and an unpacker that quietly drops
/// those produces a Lumit that will not start. `ditto` and `tar` keep them.
/// Windows has carried bsdtar (`tar.exe`) since Windows 10 1803, and it reads
/// zip files as happily as tarballs.
Future<void> _extractArchive(File archive, Directory into) async {
  final result = switch (Platform.operatingSystem) {
    'macos' =>
      await Process.run('ditto', ['-x', '-k', archive.path, into.path]),
    _ => await Process.run('tar', ['-xf', archive.path, '-C', into.path]),
  };
  if (result.exitCode != 0) {
    throw ProcessException(
      'unpack',
      [archive.path],
      result.stderr.toString().trim(),
      result.exitCode,
    );
  }
}

/// Start the swapped-in Lumit, detached, so it survives this process ending a
/// moment later.
Future<void> _relaunchLumit(File launcher) async {
  if (Platform.isMacOS) {
    // `open` starts the *bundle*, which is what makes it a proper application
    // launch — Dock icon, activation, the lot — rather than a bare process.
    final bundle = launcher.parent.parent.parent.path;
    await Process.start('open', ['-n', bundle],
        mode: ProcessStartMode.detached);
    return;
  }
  await Process.start(launcher.path, const [],
      mode: ProcessStartMode.detached, workingDirectory: launcher.parent.path);
}
