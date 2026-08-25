// Where this copy of Lumit lives, and whether it is allowed to replace itself
// (K-297).
//
// # In plain terms
//
// Chrome, VS Code and Discord update without ever showing you an installer.
// The trick is not clever patching — it is *where they live*. They install into
// your own user folder, which you can write to without an administrator, so the
// application can simply put the new files down beside the old ones and swap
// them over. An application in `Program Files` cannot: replacing anything there
// needs elevation, which is why it has to hand the job to an installer that
// asks for it.
//
// So Lumit installs per user now (K-297), and this file is the part that knows:
//
//   1. **Where the application is** — the folder on Windows and Linux, the
//      `Lumit.app` bundle on macOS.
//   2. **Whether it can be replaced from inside** — a Flatpak cannot, by design:
//      the sandbox is read-only and updating is Flatpak's job, not ours.
//   3. **How the swap is done** — the whole new version is unpacked *beside* the
//      old one and then two renames put it in place.
//
// **Why two renames rather than copying files over the top.** Renaming a folder
// is one filesystem operation: either it happened or it did not. Copying a few
// hundred files over a running application is a few hundred chances to be
// interrupted half way, leaving a Lumit that is neither the old version nor the
// new one and may not start at all. So: `Lumit` becomes `Lumit.old`, `Lumit.new`
// becomes `Lumit`, and the only moment of danger is the hair's breadth between
// the two — and if the second rename fails, the first is undone immediately,
// because the code doing it is already in memory and does not need the files on
// disk any more.
//
// The old folder is left behind deliberately: its files are still open in the
// running process, and Windows will not delete a loaded DLL. It is swept up on
// the next launch, from the new copy, when nothing is holding it.

import 'dart:io';

/// What kind of installation this is, which is what decides whether Lumit can
/// update itself and what a release attachment has to contain.
enum InstallKind {
  /// A folder of files, the usual Windows and Linux shape. The folder's
  /// *contents* are the application, and the folder keeps its path so Start
  /// Menu entries and file associations still point at it.
  folder,

  /// A macOS `.app` bundle, which is a folder the system treats as one thing.
  /// The bundle itself is what gets swapped.
  bundle,

  /// A Flatpak. The application cannot write to its own files — that is the
  /// point of the sandbox — so updating belongs to Flatpak (K-297).
  flatpak,

  /// Somewhere Lumit cannot reason about: a build tree, a test harness, an
  /// unusual layout. Never updated in place; the installer is the answer.
  unknown,
}

/// The marker written into a staged tree once every byte of it is there.
///
/// Without it an interrupted download or a half-finished unpack would look
/// exactly like a complete update waiting to be applied, and the swap would put
/// a partial Lumit in place of a working one.
const String stagedUpdateMarker = '.lumit-staged';

/// This installation: where it is, what shape it is, and what to start again
/// when the swap is done.
class InstallSite {
  final InstallKind kind;

  /// The thing that gets replaced: the install folder, or the `.app` bundle.
  final Directory root;

  /// What to run to start Lumit again, *after* the swap — so it is expressed
  /// against [root] rather than being the path this process was started from,
  /// which by then names a folder called `Lumit.old`.
  final File launcher;

  const InstallSite({
    required this.kind,
    required this.root,
    required this.launcher,
  });

  /// Where the new version is unpacked: beside the old one, so the swap is a
  /// rename rather than a copy across filesystems.
  Directory get staging => Directory('${root.path}.new');

  /// Where the old version waits to be swept up, after the swap.
  Directory get previous => Directory('${root.path}.old');

  /// Where a download is unpacked before it is known to be whole. Also a
  /// sibling, for the same reason.
  Directory get unpacking => Directory('${root.path}.unpacking');

  /// Whether Lumit may replace itself here.
  ///
  /// Two questions, both of which have to be yes: is this a shape we know how
  /// to swap, and can we actually write beside it? The second is asked of the
  /// filesystem rather than assumed from the path — an installation copied to a
  /// read-only volume or left in `Program Files` by an older installer answers
  /// no, and gets the installer route instead of a confusing failure.
  bool get replaceable {
    if (kind != InstallKind.folder && kind != InstallKind.bundle) return false;
    return _parentIsWritable;
  }

  bool get _parentIsWritable {
    try {
      final probe = File('${root.path}.write-probe');
      probe.writeAsStringSync('');
      probe.deleteSync();
      return true;
    } catch (_) {
      return false;
    }
  }

  /// Work out where we are from the running executable.
  ///
  /// [executablePath] and the two probes are injected so a test can describe an
  /// installation that does not exist on the machine running the test — which
  /// is why the path is taken apart *here* rather than with `File.parent`: that
  /// splits on whatever separator the machine running the code uses, so a
  /// Windows path handed to it on any other platform comes back as `.`, and the
  /// whole install site is then nonsense. The operating system is a parameter,
  /// so the separator has to be one too.
  static InstallSite detect({
    String? executablePath,
    String? operatingSystem,
    bool Function(String path)? isFlatpak,
  }) {
    final exe = executablePath ?? Platform.resolvedExecutable;
    final os = operatingSystem ?? Platform.operatingSystem;
    final flatpak = isFlatpak ?? (path) => File(path).existsSync();
    // Windows accepts both separators; everywhere else a backslash is an
    // ordinary character in a name and must not split anything.
    // Escaped rather than raw: a raw string cannot end in a backslash.
    final sep = os == 'windows' ? '\\' : '/';
    final split = os == 'windows' ? RegExp(r'[/\\]') : RegExp(r'/');

    String parentOf(String path) {
      final parts = path.split(split);
      if (parts.length <= 1) return path;
      parts.removeLast();
      // A leading empty part is the root slash, and joining keeps it.
      return parts.join(sep);
    }

    String join(String dir, String name) => '$dir$sep$name';

    // A Flatpak announces itself with a file the sandbox always carries. Asked
    // first, because inside the sandbox the paths below look perfectly ordinary
    // and would suggest a swap that cannot work.
    if (os == 'linux' && flatpak('/.flatpak-info')) {
      return InstallSite(
        kind: InstallKind.flatpak,
        root: Directory(parentOf(exe)),
        launcher: File(exe),
      );
    }

    if (os == 'macos') {
      // …/Lumit.app/Contents/MacOS/Lumit — the bundle is three levels up, and
      // only if the layout really is a bundle.
      final macos = parentOf(exe);
      final contents = parentOf(macos);
      final app = parentOf(contents);
      if (macos.endsWith('MacOS') &&
          contents.endsWith('Contents') &&
          app.endsWith('.app')) {
        return InstallSite(
          kind: InstallKind.bundle,
          root: Directory(app),
          launcher: File(join(join(join(app, 'Contents'), 'MacOS'),
              _name(exe))),
        );
      }
      return InstallSite(
        kind: InstallKind.unknown,
        root: Directory(parentOf(exe)),
        launcher: File(exe),
      );
    }

    final folder = parentOf(exe);
    return InstallSite(
      kind: InstallKind.folder,
      root: Directory(folder),
      launcher: File(join(folder, _name(exe))),
    );
  }

  static String _name(String path) =>
      path.split(RegExp(r'[/\\]')).where((p) => p.isNotEmpty).last;
}

/// Whether a complete new version is sitting in [site]'s staging folder,
/// waiting for the swap.
bool stagedUpdateReady(InstallSite site) =>
    File('${site.staging.path}${Platform.pathSeparator}$stagedUpdateMarker')
        .existsSync();

/// Mark a staged tree complete. Called once the unpack has finished and every
/// file has been checked, and never before.
void markStagedUpdateReady(InstallSite site) =>
    File('${site.staging.path}${Platform.pathSeparator}$stagedUpdateMarker')
        .writeAsStringSync('ready\n');

/// Put the staged version in place: two renames, and an undo if the second one
/// does not happen.
///
/// Throws when it could not be done *and* the old version is back where it was,
/// which is the only failure a caller can carry on from. Returns normally when
/// Lumit's files on disk are now the new version — at which point the running
/// process is the old one and has to be replaced by starting [InstallSite
/// .launcher] again.
void swapInStagedUpdate(InstallSite site) {
  if (!stagedUpdateReady(site)) {
    throw StateError('no complete update is staged');
  }
  // A leftover from a previous update would make the first rename fail. It is
  // only ever the version before last, and nothing is holding it now.
  _removeQuietly(site.previous);

  site.root.renameSync(site.previous.path);
  try {
    site.staging.renameSync(site.root.path);
  } catch (error) {
    // The dangerous half-second. Undo it now, while this code is in memory and
    // does not need anything on disk: leaving the application with no folder at
    // all is the one outcome worth going to lengths to avoid.
    try {
      site.previous.renameSync(site.root.path);
    } catch (_) {
      // Both renames failed, which means the filesystem is refusing us
      // entirely. Nothing here can fix that; the error below says so.
    }
    rethrow;
  }
}

/// Tidy up at start-up: finish what an interrupted update left behind.
///
/// Three states worth acting on, in the order they are checked:
///
///  * **The application folder is missing and the old one is there.** The swap
///    was cut in half — put the old version back. (Only reachable if the
///    machine stopped between the two renames, since the swap itself undoes a
///    failure; belt as well as braces.)
///  * **An old version is lying about.** The update worked and we are running
///    from the new copy, so nothing holds those files any more: delete them.
///  * **A staged tree that was never marked complete.** An abandoned download.
///    Delete it, or it will be mistaken for an update that is ready.
///
/// Never throws: this runs before the window opens, and no cleanup problem is a
/// reason for Lumit not to start.
void tidyAfterUpdate(InstallSite site) {
  try {
    if (!site.root.existsSync() && site.previous.existsSync()) {
      site.previous.renameSync(site.root.path);
      return;
    }
    if (site.previous.existsSync()) _removeQuietly(site.previous);
    if (site.staging.existsSync() && !stagedUpdateReady(site)) {
      _removeQuietly(site.staging);
    }
    _removeQuietly(site.unpacking);
  } catch (_) {
    // Best effort by design — see the doc comment.
  }
}

/// The tree an archive unpacked to, with a single wrapping folder taken off.
///
/// The Linux tarball holds `lumit-0.2.0-linux-x64/…` and the macOS archive
/// holds `Lumit.app/…`, while the Windows archive is the files themselves. One
/// rule covers all three: if the unpacked folder holds exactly one directory
/// and nothing else, that directory *is* the application.
Directory unwrapSingleFolder(Directory unpacked) {
  final entries = unpacked.listSync();
  if (entries.length == 1 && entries.single is Directory) {
    return Directory(entries.single.path);
  }
  return unpacked;
}

void _removeQuietly(Directory dir) {
  try {
    if (dir.existsSync()) dir.deleteSync(recursive: true);
  } catch (_) {
    // A folder we cannot delete costs disk space and nothing else.
  }
}
