// The library: everything a person keeps to use again in any project, and,
// with Lumit Pro, the same library on every computer signed in to the
// account.
//
// The files are the truth. Each kind is a folder, and a pass reads them all
// (off the UI thread, hashing only what changed since the last read), asks
// the server what the account holds, and works out what to do with the
// planner in library_plan.dart. What was here at the last sync is kept in a
// journal beside the profile, one per profile and computer.
//
// Nothing is ever thrown away by a sync. A file a sync replaces or deletes
// goes to the trash beside the journal first and stays there 30 days,
// downloads are checked before they are moved into place, and two changes
// to the same item wait for the person as a conflict.
//
// Reading the library is free. Without Pro, signed out, or with the
// profile's sync off, the folders are still read so the Library window can
// list them, and the server is never asked anything.

import 'dart:async';
import 'dart:convert';
import 'dart:io' hide Platform;
import 'dart:isolate';
import 'dart:math' as math;
import 'dart:typed_data';

import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/panels/graph_groups.dart' show savedGroupsChanged;
import 'package:lumit_flutter/panels/preset_save.dart' show renamePresetFile;
import 'package:lumit_flutter/src/rust/api/flare.dart' show renameFlare;
import 'package:lumit_flutter/src/rust/api/library.dart';
import 'package:lumit_flutter/src/rust/api/styles.dart';
import 'package:uuid/uuid.dart';

import '../theme/custom_theme.dart';
import '../theme/style_sheet.dart';
import '../theme/theme.dart';
import '../theme/theme_file.dart';
import '../widgets/theme_swatches.dart' show swatchesOf;
import 'account.dart';
import 'keymap.dart';
import 'library_plan.dart';
import 'profiles.dart';
import 'transfer.dart' show writeChecked;
import 'workspace.dart';
import 'package:lumit_flutter/state/platform.dart'
    if (dart.library.js_interop) 'package:lumit_flutter/state/platform_web.dart';

export 'library_plan.dart' show LibraryKind, LibraryFile;

enum LibraryStatus { off, idle, syncing, offline, failed }

enum LibraryItemState {
  synced,
  waiting,
  coming,
  notHere,
  onlyHere,
  conflict,
  tooBig,
  noRoom
}

enum LibraryResolution { keepMine, keepTheirs, keepBoth }

DateTime _at(Object? seconds) =>
    DateTime.fromMillisecondsSinceEpoch((seconds is int ? seconds : 0) * 1000);

List<LibraryFile> _filesFrom(Object? j) => [
      if (j case final List<dynamic> all)
        for (final f in all)
          if (LibraryFile.from(f) case final file? when _safe(file.name, inner: true))
            file
    ]..sort((a, b) => a.name.compareTo(b.name));

/// One version of an item, as the account keeps it.
class LibraryVersion {
  final int n;
  final List<LibraryFile> files;
  final int size;
  final String device, deviceName;
  final DateTime created;
  const LibraryVersion(this.n, this.files, this.size, this.device,
      this.deviceName, this.created);

  static LibraryVersion? from(Object? j) => j is Map && j['n'] is int
      ? LibraryVersion(
          j['n'] as int,
          _filesFrom(j['files']),
          j['size'] is int ? j['size'] as int : 0,
          '${j['device'] ?? ''}',
          '${j['deviceName'] ?? ''}',
          _at(j['created']))
      : null;

  Map<String, Object> toJson() => {
        'n': n,
        'files': [for (final f in files) f.toJson()],
        'size': size,
        'device': device,
        'deviceName': deviceName,
        'created': created.millisecondsSinceEpoch ~/ 1000,
      };
}

/// A computer signed in to the account, and the kinds it keeps.
class LibraryMachine {
  final String id, name;
  final bool me, online;
  final DateTime? seen;
  final Set<LibraryKind> kinds;
  const LibraryMachine(
      this.id, this.name, this.me, this.online, this.seen, this.kinds);
}

/// One item, here, in the account, or both.
class LibraryEntry {
  /// The server's id, or `local:<kind>/<path>` for one not in the account.
  final String key;
  final LibraryKind kind;

  /// `/` separated, inside the kind's folder.
  final String path;

  /// What it is called: a preset's own name, a font's family.
  final String name;

  /// The folder part of [path], or null.
  final String? group;

  /// One short line: a preset's effects, a font's faces, a set's binding
  /// count or the export presets' entry count, as a bare number.
  final String detail;

  /// A theme's or style's main colours as ARGB, else empty.
  final List<int> colours;
  final int size;
  final DateTime? modified;
  final LibraryItemState state;
  final bool here;

  /// The account's newest version.
  final LibraryVersion? head;

  /// Machine id to the version it holds, 0 when it does not keep it.
  final Map<String, int> held;

  /// Set while it is in Recently deleted.
  final DateTime? gone;

  /// The file or folder on this computer, when it is here.
  final String? pathHere;

  /// Not on this computer because its name, or a name inside it, is one
  /// this system can't hold. Its state is then [LibraryItemState.notHere].
  final bool cannotWrite;

  const LibraryEntry({
    required this.key,
    required this.kind,
    required this.path,
    required this.name,
    this.group,
    this.detail = '',
    this.colours = const [],
    this.size = 0,
    this.modified,
    required this.state,
    required this.here,
    this.head,
    this.held = const {},
    this.gone,
    this.pathHere,
    this.cannotWrite = false,
  });

  bool get _inAccount => !key.startsWith('local:');
}

/// Two versions of one item that both changed: this computer's, in place,
/// and the other's, fetched to [theirsPath].
class LibraryConflict {
  final LibraryEntry entry;
  final LibraryVersion theirs;
  final String theirsPath;
  final DateTime? mineChanged;

  /// The other version's one line, as [LibraryEntry.detail] is this one's.
  final String theirsDetail;
  const LibraryConflict(
      this.entry, this.theirs, this.theirsPath, this.mineChanged,
      {this.theirsDetail = ''});
}

/// The largest file the server takes.
const int _largest = 95000000;

const Map<LibraryKind, Set<String>> _extensions = {
  LibraryKind.presets: {'.lumfx', '.lummat', '.lumgrp', '.lumngrp'},
  LibraryKind.styles: {'.lumstyle'},
  LibraryKind.themes: {'.lumtheme'},
  LibraryKind.fonts: {'.ttf', '.otf', '.ttc', '.otc'},
  LibraryKind.shortcuts: {'.lumkeys'},
  LibraryKind.lenses: {'.lens'},
  LibraryKind.flares: {'.flare'},
  LibraryKind.luts: {
    '.cube', '.spi1d', '.spi3d', '.clf', '.ctf', '.cc', '.ccc', '.cdl' //
  },
  LibraryKind.templates: {'.lumt'},
};

/// The kinds whose sub-folders are groups.
const Set<LibraryKind> _deep = {
  LibraryKind.presets,
  LibraryKind.luts,
  LibraryKind.templates
};

String _base(String path) => path.split(RegExp(r'[\\/]')).last;

String _ext(String name) {
  final dot = name.lastIndexOf('.');
  return dot > 0 ? name.substring(dot).toLowerCase() : '';
}

String _stem(String path) {
  final name = _base(path);
  final dot = name.lastIndexOf('.');
  return dot > 0 ? name.substring(0, dot) : name;
}

bool _ignored(String name) {
  final lower = name.toLowerCase();
  return name.startsWith('.') ||
      name.startsWith(r'~$') ||
      lower.endsWith('.part') ||
      lower.endsWith('.tmp') ||
      lower == 'thumbs.db' ||
      lower == 'desktop.ini';
}

/// Whether a path the server sent stays inside the folder it is for. [inner]
/// allows the empty name of a single-file item. Whether this system can
/// write it is [_writableName]'s question.
bool _safe(String path, {bool inner = false}) {
  if (path.isEmpty) return inner;
  if (path.length > 400 || path.startsWith('/')) return false;
  if (path.contains(RegExp(r'[\\\x00-\x1f]'))) return false;
  return path.split('/').every((s) => s.isNotEmpty && s != '.' && s != '..');
}

final RegExp _reserved =
    RegExp(r'^(con|prn|aux|nul|com[1-9]|lpt[1-9])$', caseSensitive: false);

/// Whether one name in a path is one this system can write: on Windows not
/// `< > : " | ? *`, no trailing dot or space, and not a device's name before
/// the extension; anywhere, no more than 255 bytes.
bool _writableName(String name) {
  if (utf8.encode(name).length > 255) return false;
  if (!Platform.isWindows) return true;
  if (name.contains(RegExp(r'[<>:"|?*]'))) return false;
  if (name.endsWith('.') || name.endsWith(' ')) return false;
  return !_reserved.hasMatch(name.split('.').first.trimRight());
}

bool _single(List<LibraryFile> files) =>
    files.length == 1 && files.first.name.isEmpty;

String _metaKey(LibraryKind kind, List<LibraryFile> files) =>
    '${kind.name}:${[for (final f in files) f.hash].join(',')}';

bool _exists(String path) =>
    FileSystemEntity.typeSync(path, followLinks: false) !=
    FileSystemEntityType.notFound;

/// Move a file or folder. A rename where it can be. Where it can't, as from
/// another volume, it is copied to a hidden name beside [to] and renamed
/// into place from there, so nothing half-written is ever at [to], and only
/// then is [from] deleted.
void _move(String from, String to) {
  File(to).parent.createSync(recursive: true);
  void rename(String a, String b) => FileSystemEntity.isDirectorySync(a)
      ? Directory(a).renameSync(b)
      : File(a).renameSync(b);
  try {
    rename(from, to);
    return;
  } on FileSystemException {
    // A change of case names the same file on Windows and macOS, so the copy
    // below would end by deleting what it copied. Left as it is instead.
    if (from.toLowerCase() == to.toLowerCase()) rethrow;
    // Another volume, or held open: copied below.
  }
  final part = '${File(to).parent.path}${Platform.pathSeparator}'
      '.${_base(to)}.${DateTime.now().microsecondsSinceEpoch}.part';
  try {
    _copyTree(from, part);
    rename(part, to);
  } catch (_) {
    _remove(part);
    rethrow;
  }
  _remove(from);
}

void _copyTree(String from, String to) {
  if (FileSystemEntity.isDirectorySync(from)) {
    Directory(to).createSync(recursive: true);
    for (final e in Directory(from).listSync()) {
      _copyTree(e.path, '$to${Platform.pathSeparator}${_base(e.path)}');
    }
  } else {
    File(to).parent.createSync(recursive: true);
    File(from).copySync(to);
  }
}

void _remove(String path) {
  try {
    if (FileSystemEntity.isDirectorySync(path)) {
      Directory(path).deleteSync(recursive: true);
    } else if (_exists(path)) {
      File(path).deleteSync();
    }
  } on FileSystemException {
    // Held open somewhere. Tidied another time.
  }
}

bool _zipped(String path) {
  try {
    final head = File(path).openSync();
    final bytes = head.readSync(4);
    head.closeSync();
    return bytes.length == 4 && bytes[0] == 0x50 && bytes[1] == 0x4b;
  } on FileSystemException {
    return false;
  }
}

/// A style folder's own file follows the folder's name.
void _fixStyleFile(String folder, String was) {
  final now = _base(folder);
  if (now == was) return;
  final sep = Platform.pathSeparator;
  final file = File('$folder$sep$was.$styleFileExtension');
  if (file.existsSync()) file.renameSync('$folder$sep$now.$styleFileExtension');
}

// --- Reading the folders, off the UI thread --------------------------------

typedef _ScanJob = ({
  Map<LibraryKind, String> folders,
  Map<String, List<Object>> index,
  Set<String> known,
});

typedef _Found = ({
  LibraryKind kind,
  String path,
  List<LibraryFile> files,
  int modified,
  String abs,
});

typedef _Described = ({String name, String detail, String text});

typedef _ScanOut = ({
  List<_Found> items,
  Map<String, List<Object>> index,
  Map<String, _Described> meta,
  Set<LibraryKind> failed,
});

String _sha256Of(File file) {
  Digest? digest;
  final sink = sha256.startChunkedConversion(
      ChunkedConversionSink<Digest>.withCallback((d) => digest = d.single));
  final read = file.openSync();
  try {
    for (var chunk = read.readSync(1 << 20);
        chunk.isNotEmpty;
        chunk = read.readSync(1 << 20)) {
      sink.add(chunk);
    }
  } finally {
    read.closeSync();
  }
  sink.close();
  return '$digest';
}

/// What an item is called and its one line, from the file itself. The text
/// of a theme or style comes back too, for its colours to be read on the UI
/// thread, where the theme code lives.
_Described? _describe(LibraryKind kind, String abs, List<LibraryFile> files) {
  if (files.isEmpty) return null;
  final sep = Platform.pathSeparator;
  try {
    final file = File(kind == LibraryKind.styles && !_single(files)
        ? '$abs$sep${_base(abs)}.$styleFileExtension'
        : abs);
    if (kind == LibraryKind.fonts ||
        kind == LibraryKind.luts ||
        file.lengthSync() > 4 << 20) {
      return null;
    }
    final text = file.readAsStringSync();
    Object? json() => jsonDecode(text);
    switch (kind) {
      case LibraryKind.presets:
        final j = json() as Map;
        final boxes = j['effects'] ?? j['nodes'];
        return (
          name: j['name'] is String ? j['name'] as String : '',
          detail: [
            if (boxes is List)
              for (final e in boxes)
                if (e is Map && e['effect'] is Map)
                  if ((e['effect'] as Map)['match_name'] case final String m) m
          ].join(', '),
          text: '',
        );
      case LibraryKind.flares:
        final j = json() as Map;
        final elements = j['elements'];
        return (
          name: j['name'] is String ? j['name'] as String : '',
          detail: elements is List ? '${elements.length}' : '',
          text: '',
        );
      case LibraryKind.lenses:
        final named = RegExp(r'^name:\s*(.+)$', multiLine: true)
            .firstMatch(text)
            ?.group(1);
        return (name: named?.trim() ?? '', detail: '', text: '');
      case LibraryKind.shortcuts:
        final bindings = (json() as Map)['bindings'];
        return (
          name: '',
          detail: bindings is List
              ? '${bindings.length}'
              : bindings is Map
                  ? '${bindings.length}'
                  : '',
          text: '',
        );
      case LibraryKind.exports:
        final list = json();
        return (name: '', detail: list is List ? '${list.length}' : '', text: '');
      case LibraryKind.themes || LibraryKind.styles:
        return (name: '', detail: '', text: text);
      default:
        return null;
    }
  } catch (_) {
    // Not what its name says it is. Listed by its file's name.
    return null;
  }
}

_ScanOut _scan(_ScanJob job) {
  final sep = Platform.pathSeparator;
  final items = <_Found>[];
  final index = <String, List<Object>>{};
  final meta = <String, _Described>{};
  final failed = <LibraryKind>{};

  (LibraryFile, int) hashed(LibraryKind kind, String key, String name, File f) {
    final stat = f.statSync();
    final when = stat.modified.millisecondsSinceEpoch;
    final was = job.index['${kind.name}/$key'];
    final hash = was != null && was[0] == stat.size && was[1] == when
        ? was[2] as String
        : _sha256Of(f);
    index['${kind.name}/$key'] = [stat.size, when, hash];
    return (LibraryFile(name, hash, stat.size), when);
  }

  void add(LibraryKind kind, String path, String abs,
      List<(LibraryFile, int)> found) {
    final files = [for (final f in found) f.$1]
      ..sort((a, b) => a.name.compareTo(b.name));
    final when = found.fold(0, (latest, f) => math.max(latest, f.$2));
    items.add(
        (kind: kind, path: path, files: files, modified: when, abs: abs));
    final key = _metaKey(kind, files);
    if (!job.known.contains(key) && !meta.containsKey(key)) {
      if (_describe(kind, abs, files) case final said?) meta[key] = said;
    }
  }

  /// Every file in a style's folder, by its path inside it.
  void inside(LibraryKind kind, String item, Directory dir, String rel,
      List<(LibraryFile, int)> into) {
    for (final e in dir.listSync(followLinks: false)) {
      final name = _base(e.path);
      if (_ignored(name)) continue;
      final at = rel.isEmpty ? name : '$rel/$name';
      if (e is Directory) inside(kind, item, e, at, into);
      if (e is File) into.add(hashed(kind, '$item/$at', at, e));
    }
  }

  void walk(LibraryKind kind, Directory dir, String rel) {
    final deep = _deep.contains(kind);
    var any = false;
    for (final e in dir.listSync(followLinks: false)) {
      final name = _base(e.path);
      if (_ignored(name)) continue;
      // The footage and fonts read out of templates are made again from the
      // files, so never synced.
      if (kind == LibraryKind.templates &&
          rel.isEmpty &&
          (name == 'media' || name == 'fonts')) {
        continue;
      }
      any = true;
      final path = rel.isEmpty ? name : '$rel/$name';
      try {
        if (e is Directory) {
          if (kind == LibraryKind.styles && rel.isEmpty) {
            if (File('${e.path}$sep$name.$styleFileExtension').existsSync()) {
              final found = <(LibraryFile, int)>[];
              inside(kind, path, e, '', found);
              add(kind, path, e.path, found);
            }
          } else if (deep) {
            walk(kind, e, path);
          }
        } else if (e is File && _extensions[kind]!.contains(_ext(name))) {
          add(kind, path, e.path, [hashed(kind, path, '', e)]);
        }
      } on FileSystemException {
        // Held open, moved as it was read, or a path too long to open. Left
        // out, it would look deleted and be deleted on every computer, so the
        // whole kind sits this pass out. The change brings another.
        failed.add(kind);
      }
    }
    if (deep && rel.isNotEmpty && !any) add(kind, rel, dir.path, const []);
  }

  for (final MapEntry(key: kind, value: folder) in job.folders.entries) {
    try {
      if (kind == LibraryKind.exports) {
        final file = File(folder);
        if (file.existsSync()) {
          add(kind, _base(folder), folder,
              [hashed(kind, _base(folder), '', file)]);
        }
      } else {
        // A folder that is not there, or will not list, is left out of the
        // pass altogether, so nothing in it looks deleted.
        if (!Directory(folder).existsSync()) throw const FileSystemException();
        walk(kind, Directory(folder), '');
      }
    } on FileSystemException {
      failed.add(kind);
    }
  }
  return (items: items, index: index, meta: meta, failed: failed);
}

Future<_ScanOut> _scanOff(_ScanJob job) => Isolate.run(() => _scan(job));

// --- The library -------------------------------------------------------------

/// An item as it is on this computer now.
class _Here {
  final List<LibraryFile> files;
  final DateTime modified;
  final String abs;
  const _Here(this.files, this.modified, this.abs);
}

typedef _Meta = ({String name, String detail, List<int> colours});

/// An item changed here after the plan for it was made. The step is left,
/// and the pass plans again from a fresh read.
class _Stale implements Exception {
  const _Stale();
}

class _Undo {
  final String label;
  final Future<void> Function() undo;
  final Future<void> Function()? redo;
  const _Undo(this.label, this.undo, this.redo);
}

class LibraryState extends ChangeNotifier {
  final AccountState account;
  final ProfilesState profiles;
  final Workspace workspace;
  final KeymapState Function() keymap;

  /// Bumped so the Effects & presets panel reads its folder again.
  final ValueNotifier<int> presetsChanged;

  LibraryState({
    required this.account,
    required this.profiles,
    required this.workspace,
    required this.keymap,
    required this.presetsChanged,
  });

  /// Bumped once the engine has read the library's fonts again, for the
  /// Text panel to list families again.
  static final ValueNotifier<int> fontsChanged = ValueNotifier(0);

  late final HttpClient _http = HttpClient()
    ..connectionTimeout = const Duration(seconds: 10)
    ..userAgent = 'Lumit';

  static String get _sep => Platform.pathSeparator;

  bool _started = false, _paused = false, _gone = false;
  bool _again = false, _told = false, _redoing = false;
  Future<void>? _running;
  Timer? _settle, _beat, _later;
  final List<StreamSubscription<FileSystemEvent>> _watches = [];
  String _following = '';
  int _seq = 0;

  // --- What this computer knows ----------------------------------------------

  String _profile = '';
  final Map<LibraryKind, String> _folders = {};

  String? _jAccount;
  int _stamp = 0;
  final Map<String, LibraryItem> _items = {};
  Set<LibraryKind> _kinds = LibraryKind.values.toSet();
  final Set<LibraryKey> _onlyHere = {};
  final Set<String> _removed = {}, _got = {};
  final Map<String, String> _renamed = {};

  /// Items in conflict by id: where theirs belongs and the version it is.
  final Map<String, Map<String, dynamic>> _conflicts = {};

  Map<String, List<Object>> _index = {};
  final Map<LibraryKey, _Here> _here = {};
  Set<LibraryKind> _failed = {};

  /// Kinds whose folder was not there and was made empty to be watched. While
  /// the journal still holds items of one and the folder holds nothing, the
  /// folder went missing rather than being emptied, so the kind sits out
  /// instead of deleting everything in it on every computer.
  final Set<LibraryKind> _remade = {};
  final Map<String, _Meta> _meta = {};
  final Map<LibraryKey, LibraryItemState> _flags = {};

  bool _listed = false;
  final Map<String, LibraryItem> _remote = {};
  final Map<String, LibraryVersion> _heads = {};
  final Map<String, DateTime> _goneAt = {};
  final Map<String, Map<String, int>> _holds = {};

  // --- What the window reads -------------------------------------------------

  LibraryStatus _status = LibraryStatus.off;
  LibraryStatus get status => syncing ? _status : LibraryStatus.off;
  int _waiting = 0;
  int get waiting => _waiting;
  DateTime? _lastSynced;
  DateTime? get lastSynced => _lastSynced;
  ({int bytes, int limit, int library})? _usage;
  ({int bytes, int limit, int library})? get usage => _usage;
  List<LibraryMachine> _machines = const [];

  /// This computer's name as the account has it, which Settings ▸ Devices
  /// renames, or its network name before the account has said.
  String get _thisName {
    for (final m in _machines) {
      if (m.me && m.name.trim().isNotEmpty) return m.name.trim();
    }
    return AccountState.deviceLabel;
  }
  List<LibraryMachine> get machines => _machines;
  List<LibraryEntry> _entries = const [], _deleted = const [];
  List<LibraryEntry> get entries => _entries;
  List<LibraryEntry> get recentlyDeleted => _deleted;
  List<LibraryConflict> _conflictList = const [];
  List<LibraryConflict> get conflicts => _conflictList;

  /// Pro, signed in, and the profile's sync on.
  bool get syncing => profiles.current.sync && account.pro;

  bool kindOn(LibraryKind k) => _kinds.contains(k);

  void setKindOn(LibraryKind k, bool on) {
    on ? _kinds.add(k) : _kinds.remove(k);
    _save();
    _rebuild();
    notifyListeners();
    unawaited(syncNow());
  }

  String? folderOf(LibraryKind k) => _dir(k);

  // --- Folders and files -----------------------------------------------------

  void _readFolders() {
    _folders.clear();
    try {
      final f = libraryFolders();
      _folders.addAll({
        LibraryKind.presets: f.presets,
        LibraryKind.lenses: f.lenses,
        LibraryKind.flares: f.flares,
        LibraryKind.fonts: f.fonts,
        LibraryKind.luts: f.luts,
        LibraryKind.exports: f.exportPresets,
        LibraryKind.templates: f.templates,
      }..removeWhere((_, folder) => folder.isEmpty));
    } catch (_) {
      // No engine to ask: those kinds are left out.
    }
    _folders.addAll({
      LibraryKind.styles: Workspace.stylesFolder(),
      LibraryKind.themes: Workspace.themesFolder(),
      LibraryKind.shortcuts: Workspace.shortcutsFolder(),
    });
  }

  /// The kind's folder. Export presets are one file, so theirs is the folder
  /// that file is in.
  String? _dir(LibraryKind k) {
    final f = _folders[k];
    return f == null || k != LibraryKind.exports ? f : File(f).parent.path;
  }

  String _abs(LibraryKind k, String path) =>
      '${_dir(k)}$_sep${path.replaceAll('/', _sep)}';

  /// The sync's own working folder for a kind, beside the journal and never
  /// inside the kind's folder, so nothing that lists the library sees it:
  /// [what] is incoming, trash or conflicts.
  String _work(String what, LibraryKind k) =>
      '${Workspace.storeFile().parent.path}${_sep}profiles$_sep'
      '$_profile.library$_sep$what$_sep${k.name}';

  String _fileOf(LibraryKind k, String path, LibraryFile f) => f.name.isEmpty
      ? _abs(k, path)
      : '${_abs(k, path)}$_sep${f.name.replaceAll('/', _sep)}';

  /// The kinds this pass may touch: on here, with a folder that read.
  Set<LibraryKind> get _kindsHere =>
      {for (final k in _kinds) if (_folders.containsKey(k) && !_failed.contains(k)) k};

  /// Every item's path here, in lower case, for picking a free name.
  Set<String> _taken([Iterable<LibraryKey> more = const []]) => {
        for (final k in [..._here.keys, ...more])
          '${k.$1.name}/${k.$2}'.toLowerCase()
      };

  String? _idAt(LibraryKey at) {
    for (final i in _items.values) {
      if (i.kind == at.$1 && _samePath(_renamed[i.id] ?? i.path, at.$2)) {
        return i.id;
      }
    }
    return null;
  }

  /// Whether names differing only in case name one file, as on Windows and
  /// macOS. The planner is told the same.
  static bool get _caseless => Platform.isWindows || Platform.isMacOS;

  static bool _samePath(String a, String b) =>
      a == b || (_caseless && a.toLowerCase() == b.toLowerCase());

  /// The path an item here is read under that names the same file as [path],
  /// or null when nothing here does.
  String? _herePath(LibraryKind k, String path) {
    if (_here.containsKey((k, path))) return path;
    if (!_caseless) return null;
    for (final (kind, at) in _here.keys) {
      if (kind == k && _samePath(at, path)) return at;
    }
    return null;
  }

  /// Move an item here to the kind's trash. Where it went, or null when there
  /// was nothing there.
  String? _trash(LibraryKind k, String path) {
    final from = _abs(k, path);
    if (!_exists(from)) return null;
    final to = '${_trashSlot(k)}${path.replaceAll('/', _sep)}';
    _move(from, to);
    return to;
  }

  /// Write [bytes] beside [file] under a hidden name and rename that over
  /// it, so the file is never half there.
  void _writeOver(File file, List<int> bytes) {
    final part = File('${file.parent.path}$_sep.${_base(file.path)}.part');
    part.parent.createSync(recursive: true);
    part.writeAsBytesSync(bytes);
    part.renameSync(file.path);
  }

  /// A new folder in a kind's trash, named by when, with its separator.
  String _trashSlot(LibraryKind k) => '${_work('trash', k)}$_sep'
      '${DateTime.now().millisecondsSinceEpoch}-${_seq++}$_sep';

  /// Put [kept] back at [path], or beside it when something is there now.
  void _putBack(LibraryKind k, String kept, String path) {
    if (!_exists(kept)) return;
    final to = freePath(k, path, _taken());
    _move(kept, _abs(k, to));
    if (k == LibraryKind.styles) _fixStyleFile(_abs(k, to), _stem(path));
  }

  /// Empty each trash of anything older than 30 days, and drop the copies of
  /// conflicts that have been resolved.
  void _tidy() {
    final cut = DateTime.now()
        .subtract(const Duration(days: 30))
        .millisecondsSinceEpoch;
    for (final k in LibraryKind.values) {
      try {
        final trash = Directory(_work('trash', k));
        if (trash.existsSync()) {
          for (final e in trash.listSync()) {
            final when = int.tryParse(_base(e.path).split('-').first);
            if (when != null && when < cut) _remove(e.path);
          }
        }
        final held = Directory(_work('conflicts', k));
        if (held.existsSync()) {
          for (final e in held.listSync()) {
            if (!_conflicts.containsKey(_base(e.path))) _remove(e.path);
          }
        }
      } on FileSystemException {
        // Tidied next time.
      }
    }
  }

  // --- The journal -----------------------------------------------------------

  File _file(String what) => File('${Workspace.storeFile().parent.path}$_sep'
      'profiles$_sep$_profile.$what.json');

  Map<String, dynamic>? _readJson(File file) {
    try {
      final j = jsonDecode(file.readAsStringSync());
      return j is Map<String, dynamic> ? j : null;
    } catch (_) {
      return null;
    }
  }

  void _writeJson(File file, Object data) {
    if (_profile.isEmpty) return;
    try {
      file.parent.createSync(recursive: true);
      file.writeAsStringSync(jsonEncode(data));
    } on FileSystemException {
      // A disk that will not take it costs some hashing or a request.
    }
  }

  static LibraryKey? _keyFrom(Object? s) {
    if (s is! String || !s.contains('/')) return null;
    final kind = LibraryKind.values.asNameMap()[s.substring(0, s.indexOf('/'))];
    return kind == null ? null : (kind, s.substring(s.indexOf('/') + 1));
  }

  static String _keyOf(LibraryKey k) => '${k.$1.name}/${k.$2}';

  void _load() {
    _profile = profiles.current.id;
    final j = _readJson(_file('library')) ?? const {};
    final kinds = LibraryKind.values.asNameMap();
    _jAccount = j['account'] as String?;
    _stamp = j['stamp'] is int ? j['stamp'] as int : 0;
    _items.clear();
    if (j['items'] case final Map<String, dynamic> items) {
      for (final MapEntry(:key, :value) in items.entries) {
        if (value is Map && kinds[value['kind']] != null && value['path'] is String) {
          _items[key] = LibraryItem(
              key,
              kinds[value['kind']]!,
              value['path'] as String,
              value['n'] is int ? value['n'] as int : 0,
              _filesFrom(value['files']));
        }
      }
    }
    _kinds = j['kinds'] is List
        ? {for (final k in j['kinds'] as List) if (kinds[k] case final x?) x}
        : LibraryKind.values.toSet();
    _onlyHere
      ..clear()
      ..addAll([
        for (final k in j['onlyHere'] as List? ?? const [])
          if (_keyFrom(k) case final key?) key
      ]);
    Set<String> strings(Object? l) => {
          if (l is List)
            for (final s in l)
              if (s is String) s
        };
    _removed
      ..clear()
      ..addAll(strings(j['removed']));
    _got
      ..clear()
      ..addAll(strings(j['got']));
    _renamed
      ..clear()
      ..addAll({
        if (j['renamed'] case final Map<String, dynamic> m)
          for (final MapEntry(:key, :value) in m.entries)
            if (value is String) key: value
      });
    _conflicts
      ..clear()
      ..addAll({
        if (j['conflicts'] case final Map<String, dynamic> m)
          for (final MapEntry(:key, :value) in m.entries)
            if (value is Map<String, dynamic>) key: value
      });
    _index = {
      for (final MapEntry(:key, :value)
          in (_readJson(_file('library-index')) ?? const {}).entries)
        if (value is List && value.length == 3) key: value.cast<Object>()
    };
    _here.clear();
    _flags.clear();
    _undo.clear();
    _redo.clear();
    _told = false;
    _clearListing();
    final listing = _readJson(_file('library-listing'));
    if (listing != null && listing['account'] == _jAccount) _take(listing);
  }

  void _save() {
    _writeJson(_file('library'), {
      'account': _jAccount,
      'stamp': _stamp,
      'items': {
        for (final i in _items.values)
          i.id: {
            'kind': i.kind.name,
            'path': i.path,
            'n': i.n,
            'files': [for (final f in i.files) f.toJson()],
          }
      },
      'kinds': [for (final k in _kinds) k.name],
      'onlyHere': [for (final k in _onlyHere) _keyOf(k)],
      'removed': [..._removed],
      'got': [..._got],
      'renamed': _renamed,
      'conflicts': _conflicts,
    });
  }

  /// What was known belongs to one account. Another starts again from
  /// nothing, keeping only what this computer was told to keep to itself.
  void _startOver(String who) {
    // Another account had this profile's library. What is here stays here
    // until it is sent on purpose with Sync it again, so nothing, fonts
    // least of all, goes to an account it did not come from by itself. A
    // profile's first account takes the library as it is.
    if (_jAccount case final before? when before != who) {
      _onlyHere.addAll(_here.keys);
    }
    _jAccount = who;
    _stamp = 0;
    _items.clear();
    _removed.clear();
    _got.clear();
    _renamed.clear();
    _conflicts.clear();
    _clearListing();
    _told = false;
  }

  void _clearListing() {
    _listed = false;
    _remote.clear();
    _heads.clear();
    _goneAt.clear();
    _holds.clear();
    _machines = const [];
    _usage = null;
  }

  /// Take in what the server listed, and keep it for when it can't be asked.
  void _take(Map<String, dynamic> body, {bool keep = false}) {
    _clearListing();
    final kinds = LibraryKind.values.asNameMap();
    for (final it in body['items'] as List? ?? const []) {
      if (it is! Map || it['id'] is! String || it['path'] is! String) continue;
      final id = it['id'] as String, path = it['path'] as String;
      final kind = kinds[it['kind']];
      final head = LibraryVersion.from(it['version']);
      if (kind == null || head == null || !_safe(path)) continue;
      _heads[id] = head;
      _remote[id] = LibraryItem(id, kind, path, head.n, head.files,
          gone: it['gone'] != null);
      if (it['gone'] != null) _goneAt[id] = _at(it['gone']);
    }
    _machines = [
      for (final m in body['machines'] as List? ?? const [])
        if (m is Map && m['id'] != null)
          LibraryMachine(
            '${m['id']}',
            '${m['name'] ?? ''}',
            m['me'] == true,
            m['online'] == true,
            m['seen'] is int ? _at(m['seen']) : null,
            {for (final k in m['kinds'] as List? ?? const []) if (kinds[k] case final x?) x},
          ),
    ];
    for (final m in body['machines'] as List? ?? const []) {
      if (m is Map && m['holds'] is Map) {
        _holds['${m['id']}'] = {
          for (final MapEntry(:key, :value) in (m['holds'] as Map).entries)
            if (value is int) '$key': value
        };
      }
    }
    if (body['usage'] case final Map<dynamic, dynamic> u) {
      int number(Object? v) => v is int ? v : 0;
      _usage = (
        bytes: number(u['bytes']),
        limit: number(u['limit']),
        library: number(u['library'])
      );
    }
    if (body['stamp'] is int) _stamp = body['stamp'] as int;
    _listed = true;
    if (keep) _writeJson(_file('library-listing'), {...body, 'account': _jAccount});
  }

  // --- Starting, stopping and switching --------------------------------------

  /// Read the library and keep it in step from now on. Not called by a
  /// test, which keeps a test run off the network.
  void start() {
    if (_started) return;
    _started = true;
    _load();
    _readFolders();
    _watch();
    account.addListener(_follow);
    profiles.addListener(_follow);
    _following = _signature;
    _beat = Timer.periodic(const Duration(minutes: 10), (_) => syncNow());
    unawaited(syncNow());
  }

  String get _signature =>
      '${account.account?.id}|${account.pro}|${profiles.current.sync}';

  void _follow() {
    if (_signature == _following) return;
    _following = _signature;
    _rebuild();
    notifyListeners();
    unawaited(syncNow());
  }

  /// Let the pass under way finish, and stop, while the profile switches.
  Future<void> pause() async {
    _paused = true;
    _settle?.cancel();
    _later?.cancel();
    _unwatch();
    final running = _running;
    if (running != null) await running;
  }

  /// Pick up the profile switched to: its journal and its folders.
  void resume() {
    _paused = false;
    if (!_started || _gone) return;
    _load();
    _readFolders();
    _watch();
    // Another profile's library is in place now, so whatever lists one reads
    // it again.
    _landed(LibraryKind.values.toSet());
    _rebuild();
    notifyListeners();
    unawaited(syncNow());
  }

  void _watch() {
    _unwatch();
    for (final MapEntry(key: kind, value: folder) in _folders.entries) {
      final dir = _dir(kind)!;
      try {
        if (!Directory(dir).existsSync()) _remade.add(kind);
        Directory(dir).createSync(recursive: true);
        _watches.add(Directory(dir)
            .watch(recursive: kind != LibraryKind.exports)
            .listen((event) {
          if (_ignored(_base(event.path))) return;
          if (kind == LibraryKind.exports && _base(event.path) != _base(folder)) {
            return;
          }
          _settle?.cancel();
          _settle = Timer(const Duration(seconds: 3), () => syncNow());
        }, onError: (Object _) {}));
      } on FileSystemException {
        // Read on the beat instead.
      }
    }
  }

  void _unwatch() {
    for (final w in _watches) {
      unawaited(w.cancel());
    }
    _watches.clear();
  }

  // --- A pass ----------------------------------------------------------------

  /// Bring this computer and the account level. One pass at a time: one asked
  /// for during a pass runs after it.
  Future<void> syncNow() {
    if (!_started) return _refresh();
    if (_paused || _gone) return Future.value();
    final running = _running;
    if (running != null) {
      _again = true;
      return running;
    }
    return _running = _passes().whenComplete(() => _running = null);
  }

  /// Wait for a pass under way to finish, so an action never changes what a
  /// pass is in the middle of carrying, such as an upload that would put back
  /// an item just kept to this computer.
  Future<void> _quiet() async {
    for (var running = _running; running != null; running = _running) {
      await running;
    }
  }

  Future<void> _passes() async {
    _later?.cancel();
    for (var tries = 0; tries < 4; tries++) {
      _again = false;
      await _pass();
      if (!_again || _paused || _gone) return;
    }
    // Still something to do after four goes, such as a file being saved as
    // it was read: tried again shortly rather than on the next beat.
    _later = Timer(const Duration(seconds: 30), () => unawaited(syncNow()));
  }

  /// Read the folders again, for a library that is not being synced.
  Future<void> _refresh() async {
    if (_folders.isEmpty) _readFolders();
    await _scanNow();
    _rebuild();
    notifyListeners();
  }

  bool _current(String profile, String? who) =>
      !_gone && !_paused && _profile == profile && account.account?.id == who;

  Future<void> _pass() async {
    final profile = _profile;
    await _scanNow();
    if (_gone || _paused || _profile != profile) return;
    final who = account.account?.id;
    if (!syncing || who == null) {
      _rebuild();
      notifyListeners();
      return;
    }
    if (_jAccount != who) _startOver(who);
    _status = LibraryStatus.syncing;
    _rebuild();
    notifyListeners();
    try {
      _tidy();
      final asked = await account.call(
          'GET', '/v1/library${_listed && _stamp > 0 ? '?stamp=$_stamp' : ''}');
      if (!_current(profile, who)) return;
      if (asked['same'] != true) _take(asked, keep: true);
      final steps = _plan();
      final touched = <LibraryKind>{};
      _flags.clear();
      // Paths a step failed on. Nothing later in the pass touches them, so a
      // download never lands where a rename or an upload just failed.
      final held = <(LibraryKind, String)>{};
      Iterable<(LibraryKind, String)> pathsOf(PlanStep s) => [
            (s.kind, s.path.toLowerCase()),
            if (s.here case final here?) (s.kind, here.toLowerCase()),
          ];
      for (final s in steps) {
        if (!_current(profile, who)) return;
        if (pathsOf(s).any(held.contains)) {
          _again = true;
          continue;
        }
        var failed = true;
        try {
          if (await _carry(s)) touched.add(s.kind);
          failed = false;
        } on CloudError catch (e) {
          if (e.offline || e.status == 401 || e.status == 402) rethrow;
          final at = (s.kind, s.here ?? s.path);
          if (e.code == 'path_taken' &&
              s.action == PlanAction.upload &&
              s.id == null &&
              s.here != null) {
            // Something new here where the account keeps another item, one
            // taken off this computer: both kept, as at a first meeting.
            final aside = keepBothName(s.kind, s.here!, _thisName, _taken());
            await _renameItem(s.kind, s.here!, _stem(aside), keepId: false);
            _again = true;
          } else if (e.status == 507) {
            _flags[at] = LibraryItemState.noRoom;
          } else if (e.status == 413) {
            _flags[at] = LibraryItemState.tooBig;
          } else {
            // Changed, taken or missing on the server since it was listed,
            // or a file changed as it went: planned again from the top.
            _again = true;
          }
        } on _Stale {
          _again = true;
        } on FileSystemException {
          // A file held open elsewhere. Tried again on the next pass.
        } finally {
          if (failed) held.addAll(pathsOf(s));
        }
      }
      _landed(touched);
      // What just arrived is read like anything else here, so it is listed
      // by its own name, colours and effects at once rather than on the
      // next pass. The index keeps this to the files that changed.
      if (touched.isNotEmpty) await _scanNow();
      if (!_current(profile, who)) return;
      if (steps.isNotEmpty || !_told) {
        _take(await account.call('GET', '/v1/library'), keep: true);
        await account.call('PUT', '/v1/library/here', body: {
          'kinds': [for (final k in _kinds) k.name],
          'holds': {for (final i in _items.values) i.id: i.n},
        });
        _told = true;
      }
      _status = LibraryStatus.idle;
      _lastSynced = DateTime.now();
    } on CloudError catch (e) {
      // Without Pro there is nothing to keep in step, which is not a
      // failure, and nothing here is lost.
      _status = e.status == 402
          ? LibraryStatus.off
          : e.offline
              ? LibraryStatus.offline
              : LibraryStatus.failed;
    } catch (_) {
      _status = LibraryStatus.failed;
    } finally {
      if (!_gone && _profile == profile) {
        _save();
        _rebuild();
        notifyListeners();
      }
    }
  }

  /// What a pass would do. An item whose name this system can't hold is
  /// never written here, and is listed as not on this computer.
  List<PlanStep> _plan() => [
        for (final s in planLibrary(
          local: {
            for (final MapEntry(:key, :value) in _here.entries)
              key: value.files
          },
          journal: _items,
          remote: _remote,
          kindsOn: _kindsHere,
          machine: _thisName,
          onlyHere: _onlyHere,
          removed: _removed,
          // Fetched one by one, but only where their kind's folder read
          // this time, or a folder that failed would read as them deleted.
          got: {
            for (final id in _got)
              if ((_items[id] ?? _remote[id])?.kind case final k?
                  when _folders.containsKey(k) && !_failed.contains(k))
                id
          },
          held: _conflicts.keys.toSet(),
          renamed: _renamed,
          foldCase: _caseless,
        ))
          if (!const {
                PlanAction.download,
                PlanAction.renameHere,
                PlanAction.conflict
              }.contains(s.action) ||
              _writable(s.kind, s.path, s.files))
            s
      ];

  /// Whether this system can hold the item at [path] with [files] in it.
  bool _writable(LibraryKind k, String path, List<LibraryFile> files) {
    if (_dir(k) == null) return false;
    final longest = Platform.isWindows ? 259 : 1023;
    for (final p in [
      path,
      for (final f in files)
        if (f.name.isNotEmpty) '$path/${f.name}'
    ]) {
      if (!p.split('/').every(_writableName) || _abs(k, p).length > longest) {
        return false;
      }
    }
    return true;
  }

  /// Whether the item at [path] is still as the last read found it: the
  /// same files, each the same size and time. Anything else was saved since
  /// the plan was made.
  bool _unchanged(LibraryKind k, String asked) {
    final path = _herePath(k, asked) ?? asked;
    final here = _here[(k, path)];
    final abs = _abs(k, path);
    if (here == null) return !_exists(abs);
    try {
      if (!_single(here.files)) {
        var count = 0;
        if (FileSystemEntity.isDirectorySync(abs)) {
          for (final e in Directory(abs).listSync(recursive: true)) {
            final inside = e.path.substring(abs.length + 1);
            if (e is File && !inside.split(RegExp(r'[\\/]')).any(_ignored)) {
              count++;
            }
          }
        }
        if (count != here.files.length) return false;
      }
      for (final f in here.files) {
        final seen = _index['${k.name}/$path'
            '${f.name.isEmpty ? '' : '/${f.name}'}'];
        final stat = File(_fileOf(k, path, f)).statSync();
        if (seen == null ||
            stat.type == FileSystemEntityType.notFound ||
            stat.size != seen[0] ||
            stat.modified.millisecondsSinceEpoch != seen[1]) {
          return false;
        }
      }
      return true;
    } on FileSystemException {
      return false;
    }
  }

  /// [_trash], for a sync, which only ever moves what the plan saw: an item
  /// saved since is left where it is and the pass planned again.
  String? _trashSeen(LibraryKind k, String path) {
    if (_exists(_abs(k, path)) && !_unchanged(k, path)) throw const _Stale();
    return _trash(k, path);
  }

  Future<void> _scanNow() async {
    final profile = _profile;
    final folders = {
      for (final MapEntry(:key, :value) in _folders.entries) key: value
    };
    final out = await _scanOff((
      folders: folders,
      index: _index,
      known: _meta.keys.toSet(),
    ));
    if (_gone || _profile != profile) return;
    _here
      ..clear()
      ..addAll({
        for (final f in out.items)
          (f.kind, f.path): _Here(f.files,
              DateTime.fromMillisecondsSinceEpoch(f.modified), f.abs)
      });
    _index = out.index;
    _failed = out.failed;
    for (final k in _remade.toList()) {
      final empty = !_here.keys.any((at) => at.$1 == k);
      final known = _items.values.any((i) => i.kind == k);
      if (empty && known) {
        _failed.add(k);
      } else {
        _remade.remove(k);
      }
    }
    _writeJson(_file('library-index'), _index);
    // Kept only for what is here or in the account, so it never outgrows
    // the library.
    final live = {
      for (final MapEntry(key: (k, _), :value) in _here.entries)
        _metaKey(k, value.files),
      for (final r in _remote.values) _metaKey(r.kind, r.files),
    };
    _meta.removeWhere((key, _) => !live.contains(key));
    for (final MapEntry(:key, :value) in out.meta.entries) {
      _meta[key] = _named(key, value);
    }
    for (final MapEntry(key: (kind, _), value: here) in _here.entries) {
      final key = _metaKey(kind, here.files);
      if (kind != LibraryKind.fonts || _meta.containsKey(key)) continue;
      try {
        final font = await libraryFontInfo(path: here.abs);
        _meta[key] = (
          name: font?.family ?? '',
          detail: font?.faces.join(', ') ?? '',
          colours: const []
        );
      } catch (_) {
        _meta[key] = (name: '', detail: '', colours: const []);
      }
    }
    // The other side of each conflict, fetched beside the journal.
    for (final MapEntry(key: id, value: c) in _conflicts.entries) {
      final theirs = LibraryVersion.from(c['version']);
      final kind = _remote[id]?.kind;
      if (theirs == null || kind == null) continue;
      final key = _metaKey(kind, theirs.files);
      if (_meta.containsKey(key)) continue;
      final at = _theirsAt(kind, id, '${c['path'] ?? ''}');
      if (_describe(kind, at, theirs.files) case final said?) {
        _meta[key] = _named(key, said);
      }
    }
  }

  /// The name and colours of a theme or style, read with the theme's own
  /// code.
  _Meta _named(String key, _Described said) {
    if (said.text.isEmpty) {
      return (name: said.name, detail: said.detail, colours: const []);
    }
    // The eight colours Settings ▸ Appearance shows a theme by.
    List<int> argb(LumitTheme t) => [for (final c in swatchesOf(t)) c.toARGB32()];
    try {
      if (key.startsWith('${LibraryKind.themes.name}:')) {
        final theme = readThemeFile(said.text).theme;
        if (theme == null) return (name: '', detail: '', colours: const []);
        return (
          name: theme.name,
          detail: '',
          colours: argb(theme.build(ThemeShape.studio))
        );
      }
      final sheet = StyleSheet.parse(said.text, name: '');
      final scheme =
          sheet.baseName == null ? null : schemeNamed(sheet.baseName!);
      final base = scheme ?? LumitColorScheme.dark;
      final styled = CustomTheme(
          name: '',
          mode: base.mode,
          colours: sheet.colours(base.build()),
          base: scheme);
      return (name: '', detail: '', colours: argb(styled.build(ThemeShape.studio)));
    } catch (_) {
      return (name: '', detail: '', colours: const []);
    }
  }

  /// Carry out one step. True when files here changed.
  Future<bool> _carry(PlanStep s) async {
    final id = s.id;
    void record(String path, int n, List<LibraryFile> files) {
      _items[id!] = LibraryItem(id, s.kind, path, n, files);
      _renamed.remove(id);
    }

    switch (s.action) {
      case PlanAction.forget:
        _items.remove(id);
        _renamed.remove(id);
        return false;
      case PlanAction.record:
        record(s.path, s.base, s.files);
        return false;
      case PlanAction.renameHere:
        // Saved since it was read, or something new where it goes: planned
        // again rather than moved over.
        if (!_unchanged(s.kind, s.here!)) throw const _Stale();
        if (!_samePath(s.here!, s.path) && _exists(_abs(s.kind, s.path))) {
          throw const _Stale();
        }
        _move(_abs(s.kind, s.here!), _abs(s.kind, s.path));
        record(s.path, s.base, s.files);
        return true;
      case PlanAction.renameThere:
        await _put(s.kind, id!, s.path, s.base, s.files);
        return false;
      case PlanAction.keepBoth:
        if (!_unchanged(s.kind, s.here!)) throw const _Stale();
        if (await _renameItem(s.kind, s.here!, _stem(s.path), keepId: false) ==
            null) {
          // The app's own rename refused (a preset that will not read, a
          // name taken): the file moves as it is. If that fails too the
          // step fails, and the download after it is never carried.
          _move(_abs(s.kind, s.here!), _abs(s.kind, s.path));
        }
        // Goes up as a new item on the pass after, once read again.
        _again = true;
        return true;
      case PlanAction.deleteHere:
        _trashSeen(s.kind, s.here ?? s.path);
        _items.remove(id);
        _renamed.remove(id);
        return true;
      case PlanAction.deleteThere:
        await account
            .call('DELETE', '/v1/library/items/$id?base=${s.base}');
        _items.remove(id);
        _renamed.remove(id);
        return false;
      case PlanAction.download:
        await _land(s.kind, id!, s.files, s.path, here: s.here);
        record(s.path, s.base, s.files);
        return true;
      case PlanAction.conflict:
        final to = _theirsAt(s.kind, id!, s.path);
        _remove(File(to).parent.path);
        await _fetch(s.files, to);
        // Read like anything here, so its card says what is in it.
        final key = _metaKey(s.kind, s.files);
        if (!_meta.containsKey(key)) {
          if (_describe(s.kind, to, s.files) case final said?) {
            _meta[key] = _named(key, said);
          }
        }
        _conflicts[id] = {
          'path': s.path,
          'version': (_heads[id] ??
                  LibraryVersion(s.base, s.files, 0, '', '', DateTime.now()))
              .toJson(),
        };
        return false;
      case PlanAction.merge:
        return _merge(s);
      case PlanAction.upload || PlanAction.revive:
        await _upload(s.kind, id ?? const Uuid().v4().replaceAll('-', ''),
            s.path, s.base, s.files);
        return false;
    }
  }

  String _theirsAt(LibraryKind k, String id, String path) =>
      '${_work('conflicts', k)}$_sep$id$_sep${_base(path)}';

  /// Send what the server lacks of [files], then make them the item's newest
  /// version.
  Future<void> _upload(LibraryKind k, String id, String path, int base,
      List<LibraryFile> files) async {
    if (files.any((f) => f.size > _largest)) {
      throw const CloudError('too_large', 413);
    }
    // Saved again since it was read: what was hashed is not what would go.
    if (!_unchanged(k, path)) throw const _Stale();
    final hashes = {for (final f in files) f.hash}.toList();
    final missing = <String>{};
    for (var i = 0; i < hashes.length; i += 1000) {
      final answer = await account.call('POST', '/v1/library/blobs', body: {
        'hashes': hashes.sublist(i, math.min(i + 1000, hashes.length)),
      });
      missing.addAll([
        for (final h in answer['missing'] as List? ?? const [])
          if (h is String) h
      ]);
    }
    for (final f in files) {
      if (!missing.remove(f.hash)) continue;
      final sent = await _exchange('PUT', '/v1/library/blobs/${f.hash}',
          send: File(_fileOf(k, path, f)).openRead(), length: f.size);
      await sent.drain<void>();
    }
    await _put(k, id, path, base, files);
  }

  Future<void> _put(LibraryKind k, String id, String path, int base,
      List<LibraryFile> files) async {
    final answer = await account.call('PUT', '/v1/library/items/$id', body: {
      'kind': k.name,
      'path': path,
      'base': base,
      'files': [for (final f in files) f.toJson()],
    });
    final item = answer['item'];
    final n = item is Map && item['head'] is int ? item['head'] as int : base + 1;
    _items[id] = LibraryItem(id, k, path, n, files);
    _renamed.remove(id);
  }

  /// Fetch [files] to [to]: the file itself for a one-file item, else a
  /// folder holding them. Each is checked against its hash as it lands.
  Future<void> _fetch(List<LibraryFile> files, String to) async {
    if (_single(files)) return _blob(files.first, to);
    Directory(to).createSync(recursive: true);
    for (final f in files) {
      await _blob(f, '$to$_sep${f.name.replaceAll('/', _sep)}');
    }
  }

  Future<void> _blob(LibraryFile f, String to) async {
    File(to).parent.createSync(recursive: true);
    final got = await _exchange('GET', '/v1/library/blobs/${f.hash}');
    await writeChecked(got.timeout(const Duration(seconds: 30)), f.hash, to);
  }

  /// Fetch an item into the kind's incoming folder, then swap it in whole,
  /// with whatever was in the way moved to the trash first.
  Future<void> _land(LibraryKind k, String id, List<LibraryFile> files,
      String path, {String? here}) async {
    final stage = '${_work('incoming', k)}$_sep$id';
    _remove(stage);
    try {
      final item = '$stage${_sep}item';
      await _fetch(files, item);
      if (files.isEmpty) Directory(item).createSync(recursive: true);
      // Both checked before either moves, so a stale plan moves nothing.
      for (final p in [if (here != null) here, path]) {
        if (_exists(_abs(k, p)) && !_unchanged(k, p)) throw const _Stale();
      }
      if (here != null) _trash(k, here);
      _trash(k, path);
      _move(item, _abs(k, path));
    } finally {
      _remove(stage);
    }
  }

  Future<Uint8List> _bytes(LibraryFile f) async {
    final got = await _exchange('GET', '/v1/library/blobs/${f.hash}');
    final bytes = BytesBuilder(copy: false);
    await got.timeout(const Duration(seconds: 30)).forEach(bytes.add);
    final all = bytes.takeBytes();
    if ('${sha256.convert(all)}' != f.hash) throw const CloudError('bad_hash');
    return all;
  }

  /// Export presets changed on both sides: merged entry by entry, with this
  /// computer's file kept in the trash.
  Future<bool> _merge(PlanStep s) async {
    final here = s.here ?? s.path;
    final file = File(_abs(s.kind, here));
    if (!_unchanged(s.kind, here)) throw const _Stale();
    final mine = await file.readAsString();
    final base = s.was.isEmpty ? null : utf8.decode(await _bytes(s.was.first));
    final theirs = utf8.decode(await _bytes(s.files.first));
    final merged = mergeExports(base, mine, theirs);
    if (merged == null) {
      return _carry(PlanStep(PlanAction.conflict, s.kind, s.path,
          id: s.id, here: here, base: s.base, files: s.files));
    }
    final kept = '${_trashSlot(s.kind)}${_base(here)}';
    File(kept).parent.createSync(recursive: true);
    file.copySync(kept);
    final bytes = utf8.encode(merged);
    _writeOver(file, bytes);
    final merges = [LibraryFile('', '${sha256.convert(bytes)}', bytes.length)];
    // What was just written is what is here now, so the upload's check
    // against the last read passes rather than calling it stale every pass.
    final stat = file.statSync();
    _index['${s.kind.name}/$here'] = [
      stat.size,
      stat.modified.millisecondsSinceEpoch,
      merges.single.hash,
    ];
    _here[(s.kind, here)] = _Here(merges, stat.modified, file.path);
    await _upload(s.kind, s.id!, here, s.base, merges);
    return true;
  }

  /// Bytes to or from the server as the signed-in account. Throws a
  /// [CloudError] for anything but a 2xx answer. As BackupState's.
  Future<HttpClientResponse> _exchange(String method, String path,
      {Stream<List<int>>? send, int length = 0}) async {
    final token = await account.accessToken();
    if (token == null) throw const CloudError('signed_out', 401);
    final HttpClientRequest request;
    try {
      request = await _http
          .openUrl(method, Uri.parse('$cloudUrl$path'))
          .timeout(const Duration(seconds: 15));
    } catch (_) {
      throw const CloudError('offline');
    }
    request.headers.set(HttpHeaders.authorizationHeader, 'Bearer $token');
    Future<HttpClientResponse> exchange() async {
      if (send != null) {
        request.headers.contentType = ContentType.binary;
        request.contentLength = length;
        await request.addStream(send);
      }
      return request.close();
    }

    final HttpClientResponse response;
    try {
      // A minute, and longer for a file: as long as it would take on a line
      // that manages sixteen kilobytes a second.
      response = await exchange().timeout(
          Duration(seconds: 60 + length ~/ 16384), onTimeout: () {
        request.abort();
        throw const CloudError('offline');
      });
    } on CloudError {
      rethrow;
    } catch (_) {
      // Connected, and then the bytes would not go: most often a file that
      // changed size as it was read. That item is tried again; the pass
      // carries on.
      throw const CloudError('send_failed');
    }
    if (response.statusCode ~/ 100 == 2) return response;
    var code = 'unknown';
    try {
      final said = jsonDecode(await utf8.decodeStream(response));
      if (said is Map && said['error'] is String) code = said['error'] as String;
    } catch (_) {
      // Not JSON: an error page from something in between.
    }
    throw CloudError(code, response.statusCode);
  }

  /// Tell whatever reads a kind's files that they changed.
  void _landed(Set<LibraryKind> kinds) {
    for (final k in kinds) {
      try {
        switch (k) {
          case LibraryKind.presets:
            presetsChanged.value++;
            savedGroupsChanged.value++;
          case LibraryKind.styles:
            workspace.reloadStyles();
          case LibraryKind.themes:
            workspace.reloadThemes();
          case LibraryKind.fonts:
            unawaited(libraryFontsChanged()
                .then((_) => fontsChanged.value++, onError: (Object _) {}));
          case LibraryKind.shortcuts:
            keymap().reloadSets();
          default:
        }
      } catch (_) {
        // Nothing listening yet, as in a test.
      }
    }
  }

  // --- What the window shows -------------------------------------------------

  String _nameOf(LibraryKind k, String path, List<LibraryFile> files) {
    final said = _meta[_metaKey(k, files)]?.name ?? '';
    if (said.isNotEmpty) return said;
    final folder = files.isEmpty || !_single(files);
    return folder ? _base(path) : _stem(path);
  }

  void _rebuild() {
    final shown = syncing && _listed && _jAccount == account.account?.id;
    final steps = syncing ? _plan() : const <PlanStep>[];
    const up = {
      PlanAction.upload,
      PlanAction.revive,
      PlanAction.renameThere,
      PlanAction.deleteThere,
      PlanAction.keepBoth,
      PlanAction.merge,
    };
    _waiting = steps.where((s) => up.contains(s.action)).length;
    final coming = {
      for (final s in steps)
        if (s.action == PlanAction.download && s.id != null) s.id!
    };
    Map<String, int> held(String? id) => id == null || !shown
        ? const {}
        : {
            for (final m in _machines)
              m.id: m.me ? (_items[id]?.n ?? 0) : (_holds[m.id]?[id] ?? 0)
          };
    LibraryEntry entry(String key, LibraryKind k, String path,
        List<LibraryFile> files, LibraryItemState state,
        {_Here? here, String? id, bool cannotWrite = false}) {
      final meta = _meta[_metaKey(k, files)];
      final cut = path.lastIndexOf('/');
      return LibraryEntry(
        key: key,
        kind: k,
        path: path,
        name: _nameOf(k, path, files),
        group: cut < 0 ? null : path.substring(0, cut),
        detail: meta?.detail ?? '',
        colours: meta?.colours ?? const [],
        size: files.fold(0, (sum, f) => sum + f.size),
        modified: here?.modified ?? (id == null ? null : _heads[id]?.created),
        state: state,
        here: here != null,
        head: id == null || !shown ? null : _heads[id],
        held: held(id),
        gone: id == null ? null : _goneAt[id],
        pathHere: here?.abs,
        cannotWrite: cannotWrite,
      );
    }

    String fold(String path) => _caseless ? path.toLowerCase() : path;
    final idAt = {
      for (final i in _items.values)
        (i.kind, fold(_renamed[i.id] ?? i.path)): i.id
    };
    final list = <LibraryEntry>[];
    final listed = <String>{};
    for (final MapEntry(key: at, value: here) in _here.entries) {
      final (k, path) = at;
      final id = idAt[(k, fold(path))];
      if (id != null) listed.add(id);
      final j = id == null ? null : _items[id];
      // Without sync nothing is in step or out of it, so nothing wears a
      // mark: the library is simply here.
      final state = !syncing
          ? LibraryItemState.synced
          : id != null && _conflicts.containsKey(id)
          ? LibraryItemState.conflict
          : _flags[at] ??
              (_onlyHere.contains(at) ||
                      (!_kinds.contains(k) && !_got.contains(id))
                  ? LibraryItemState.onlyHere
                  : j == null ||
                          !sameFiles(here.files, j.files) ||
                          _renamed.containsKey(id)
                      ? LibraryItemState.waiting
                      : coming.contains(id)
                          ? LibraryItemState.coming
                          : LibraryItemState.synced);
      list.add(entry(id ?? 'local:${k.name}/$path', k, path, here.files, state,
          here: here, id: id));
    }
    final deleted = <LibraryEntry>[];
    if (shown) {
      for (final r in _remote.values) {
        if (r.gone) {
          deleted.add(entry(r.id, r.kind, r.path, r.files,
              LibraryItemState.notHere, id: r.id));
        } else if (!listed.contains(r.id)) {
          list.add(entry(
              r.id,
              r.kind,
              r.path,
              r.files,
              coming.contains(r.id)
                  ? LibraryItemState.coming
                  : LibraryItemState.notHere,
              id: r.id,
              cannotWrite: _dir(r.kind) != null &&
                  !_writable(r.kind, r.path, r.files)));
        }
      }
    }
    int byPath(LibraryEntry a, LibraryEntry b) {
      final k = a.kind.index - b.kind.index;
      return k != 0 ? k : a.path.toLowerCase().compareTo(b.path.toLowerCase());
    }

    _entries = List.unmodifiable(list..sort(byPath));
    _deleted = List.unmodifiable(deleted..sort(byPath));
    _conflictList = List.unmodifiable([
      for (final e in _entries)
        if (_conflicts[e.key] case final c?)
          if (LibraryVersion.from(c['version']) case final theirs?)
            LibraryConflict(
                e,
                theirs,
                _theirsAt(e.kind, e.key, '${c['path'] ?? e.path}'),
                e.modified,
                theirsDetail:
                    _meta[_metaKey(e.kind, theirs.files)]?.detail ?? ''),
    ]);
  }

  // --- Undo --------------------------------------------------------------------

  final List<_Undo> _undo = [], _redo = [];

  bool get canUndo => _undo.isNotEmpty;
  bool get canRedo => _redo.isNotEmpty;

  /// What Undo and Redo would do, as `<action>:<names>`, the action being
  /// one of delete, rename, restore, keepOnlyHere, removeHere, get, resolve.
  String? get undoing => _undo.lastOrNull?.label;
  String? get redoing => _redo.lastOrNull?.label;

  void _push(String action, List<LibraryEntry> about,
      Future<void> Function() undo, Future<void> Function()? redo) {
    _undo.add(_Undo('$action:${[for (final e in about) e.name].join(', ')}',
        undo, redo));
    if (_undo.length > 50) _undo.removeAt(0);
    if (!_redoing) _redo.clear();
    notifyListeners();
  }

  Future<void> undo() async {
    await _quiet();
    if (_undo.isEmpty) return;
    final step = _undo.removeLast();
    await step.undo();
    if (step.redo != null) _redo.add(step);
    _save();
    notifyListeners();
    await syncNow();
  }

  Future<void> redo() async {
    await _quiet();
    if (_redo.isEmpty) return;
    final step = _redo.removeLast();
    _redoing = true;
    try {
      await step.redo!();
    } finally {
      _redoing = false;
    }
  }

  // --- Actions -------------------------------------------------------------------

  /// Make the account's item [id] its version [n] again, which also brings
  /// back a deleted one.
  Future<void> _restoreTo(String id, int n) =>
      account.call('POST', '/v1/library/items/$id/restore', body: {'n': n});

  Future<List<LibraryEntry>> add(List<String> paths) async {
    final landed = <LibraryKey>{};
    for (final p in paths) {
      try {
        await _add(p, landed);
      } catch (_) {
        // Not one the library takes, or one that would not copy. The rest
        // go in all the same.
      }
    }
    _landed({for (final k in landed) k.$1});
    await syncNow();
    return [
      for (final e in _entries)
        if (landed.contains((e.kind, e.path))) e
    ];
  }

  Future<void> _add(String p, Set<LibraryKey> landed) async {
    final ext = _ext(_base(p));
    if (ext == '.lumlibrary') return _unpack(p, landed);
    if (ext == '.lumstyle') {
      final at = await _addStyle(p, landed);
      if (at != null) landed.add((LibraryKind.styles, at));
      return;
    }
    if (ext == '.lumtheme') {
      final before = _top(LibraryKind.themes);
      if (await workspace.importThemeFile(p) != null) return;
      final at = _newIn(LibraryKind.themes, before);
      if (at != null) landed.add((LibraryKind.themes, at));
      return;
    }
    final kind = [
      for (final MapEntry(:key, :value) in _extensions.entries)
        if (value.contains(ext)) key
    ].firstOrNull;
    if (kind == null || _dir(kind) == null) return;
    if (kind == LibraryKind.fonts && await libraryFontInfo(path: p) == null) {
      return;
    }
    final at = freePath(kind, _base(p), _taken(landed));
    _bring(kind, p, at);
    landed.add((kind, at));
  }

  /// Copy [from] in as the item at [at]: whole into the incoming folder
  /// first, then moved into place, so a copy cut off leaves nothing there.
  void _bring(LibraryKind k, String from, String at) {
    final stage = '${_work('incoming', k)}${_sep}in-${_seq++}';
    try {
      _copyTree(from, stage);
      _move(stage, _abs(k, at));
    } finally {
      _remove(stage);
    }
  }

  /// A style through the app's own style import: a package is unpacked and
  /// a style in a folder of its own is packed and unpacked, which is what
  /// checks it, then put in the styles folder under a free name.
  Future<String?> _addStyle(String p, Set<LibraryKey> landed) async {
    const k = LibraryKind.styles;
    final stage = '${_work('incoming', k)}${_sep}add-${_seq++}';
    try {
      var package = _zipped(p) ? p : null;
      if (package == null &&
          _base(File(p).parent.path).toLowerCase() == _stem(p).toLowerCase()) {
        package = '$stage.zip';
        Directory(stage).createSync(recursive: true);
        final made =
            await styleMakePackage(style: p, wholeFolder: true, out: package);
        if (made is! BridgeStyleOutcome_Ready) return null;
      }
      if (package == null) {
        final at = freePath(k, _base(p), _taken(landed));
        _bring(k, p, at);
        return at;
      }
      final done = await styleInstallPackage(path: package, styles: stage);
      if (done is! BridgeStyleOutcome_Ready) return null;
      final at = freePath(k, done.name, _taken(landed));
      _move('$stage$_sep${done.name}', _abs(k, at));
      _fixStyleFile(_abs(k, at), done.name);
      return at;
    } finally {
      _remove(stage);
      _remove('$stage.zip');
    }
  }

  /// The names at the top of a kind's folder, to find the one an app routine
  /// that names its own file has just made.
  Set<String> _top(LibraryKind k) {
    try {
      return {for (final e in Directory(_dir(k)!).listSync()) _base(e.path)};
    } catch (_) {
      return const {};
    }
  }

  String? _newIn(LibraryKind k, Set<String> before) =>
      _top(k).where((n) => !before.contains(n) && !_ignored(n)).firstOrNull;

  Future<void> _unpack(String file, Set<LibraryKey> landed) async {
    final tmp = await Directory.systemTemp.createTemp('lumit-library-');
    try {
      for (final it in await libraryUnpack(file: file, into: tmp.path)) {
        final kind = LibraryKind.values.asNameMap()[it.kind];
        if (kind == null || _dir(kind) == null || !_safe(it.name)) continue;
        if (kind == LibraryKind.exports) {
          // One list: what it holds is added to this one's, by name.
          final mine = File(_folders[kind]!);
          final ours = mine.existsSync() ? mine.readAsStringSync() : '[]';
          final merged =
              mergeExports(null, ours, File(it.path).readAsStringSync());
          if (merged == null) continue;
          if (mine.existsSync()) {
            _copyTree(mine.path, '${_trashSlot(kind)}${_base(mine.path)}');
          }
          _writeOver(mine, utf8.encode(merged));
          landed.add((kind, _base(mine.path)));
          continue;
        }
        final at = freePath(kind, it.name, _taken(landed));
        _bring(kind, it.path, at);
        if (kind == LibraryKind.styles) {
          _fixStyleFile(_abs(kind, at), _stem(it.name));
        }
        landed.add((kind, at));
      }
    } finally {
      try {
        await tmp.delete(recursive: true);
      } on FileSystemException {
        // The system's temp folder is tidied by the system.
      }
    }
  }


  Future<void> delete(List<LibraryEntry> e) async {
    await _quiet();
    final back = <Future<void> Function()>[];
    for (final x in e) {
      if (x.here) {
        final kept = _trash(x.kind, x.path);
        if (kept != null) back.add(() async => _putBack(x.kind, kept, x.path));
      } else if (x._inAccount && syncing) {
        await account.call(
            'DELETE', '/v1/library/items/${x.key}?base=${x.head?.n ?? 0}');
      }
      if (x._inAccount && syncing) {
        back.add(() async {
          if (_remote[x.key]?.gone ?? !x.here) {
            await _restoreTo(x.key, x.head?.n ?? _remote[x.key]?.n ?? 0);
          }
        });
      }
    }
    _landed({for (final x in e) x.kind});
    _push('delete', e, () async {
      for (final b in back) {
        await b();
      }
      _landed({for (final x in e) x.kind});
    }, () => delete(e));
    await syncNow();
  }

  /// Rename an item here: its file or folder, the name inside the file for
  /// the kinds that keep one, through the app's own rename where it has one.
  /// Where it went, or null when it could not be renamed.
  Future<String?> _renameItem(LibraryKind k, String path, String name,
      {bool keepId = true}) async {
    final from = _abs(k, path);
    final cut = path.lastIndexOf('/');
    final group = path.substring(0, cut + 1);
    final file = path.substring(cut + 1);
    final folder = FileSystemEntity.isDirectorySync(from);
    final ext = folder ? '' : file.substring(file.length - _ext(file).length);
    final stem = name.replaceAll(RegExp(r'[\\/:*?"<>|\x00-\x1f]'), '-').trim();
    if (stem.isEmpty || stem.startsWith('.')) return null;
    final here = _here[(k, path)];
    final was = here == null ? _stem(path) : _nameOf(k, path, here.files);
    var to = '$group$stem$ext';
    if (to == path) return path;
    if (to.toLowerCase() != path.toLowerCase()) to = freePath(k, to, _taken());
    final before = _top(k);
    String? landed;
    switch (k) {
      case LibraryKind.presets when !folder:
        landed = await renamePresetFile(workspace, from, _abs(k, to),
                from: was, to: name.trim())
            ? to
            : null;
      case LibraryKind.themes:
        workspace.renameCustomTheme(was, name.trim());
        landed = _newIn(k, before);
      case LibraryKind.shortcuts:
        final now = keymap().renameSet(_stem(file), stem);
        landed = now == _stem(file) ? null : '$group$now$ext';
      case LibraryKind.flares:
        try {
          await renameFlare(name: was, newName: name.trim());
          landed = _newIn(k, before);
        } catch (_) {
          landed = null;
        }
      case LibraryKind.lenses:
        // The name inside is written in place, then the file moves, so a
        // change of case only, which names the same file, loses nothing.
        final text = File(from).readAsStringSync();
        final line = RegExp(r'^name:.*$', multiLine: true);
        File(from).writeAsStringSync(line.hasMatch(text)
            ? text.replaceFirst(line, 'name: ${name.trim()}')
            : 'name: ${name.trim()}\n$text');
        _move(from, _abs(k, to));
        landed = to;
      default:
        _move(from, _abs(k, to));
        if (k == LibraryKind.styles) {
          if (folder) _fixStyleFile(_abs(k, to), _base(path));
          if (workspace.customStyle == _stem(file)) {
            workspace.setCustomStyle(_stem(to));
          }
        }
        landed = to;
    }
    if (landed == null) return null;
    if (keepId) {
      if (_idAt((k, path)) case final id?) _renamed[id] = landed;
    }
    if (_onlyHere.remove((k, path))) _onlyHere.add((k, landed));
    _save();
    return landed;
  }

  Future<void> rename(LibraryEntry e, String name) async {
    await _quiet();
    if (!e.here || name.trim().isEmpty || name.trim() == e.name) return;
    final to = await _renameItem(e.kind, e.path, name);
    if (to == null) return;
    _landed({e.kind});
    final was = e.name;
    _push('rename', [e], () async {
      await _renameItem(e.kind, to, was);
      _landed({e.kind});
    }, () => rename(e, name));
    await syncNow();
  }

  Future<List<LibraryVersion>> history(LibraryEntry e) async {
    if (!e._inAccount) return const [];
    final answer = await account.call('GET', '/v1/library/items/${e.key}');
    return [
      for (final v in answer['versions'] as List? ?? const [])
        if (LibraryVersion.from(v) case final version?) version
    ];
  }

  Future<void> restore(LibraryEntry e, LibraryVersion v) async {
    await _quiet();
    final was = e.head?.n;
    await _restoreTo(e.key, v.n);
    _push('restore', [e], () async {
      if (was != null) await _restoreTo(e.key, was);
    }, () => restore(e, v));
    await syncNow();
  }

  /// [v] of [e] as a new item beside it.
  Future<void> saveCopy(LibraryEntry e, LibraryVersion v) async {
    await _quiet();
    if (_dir(e.kind) == null) return;
    final at = freePath(e.kind, e.path, _taken());
    final stage = '${_work('incoming', e.kind)}${_sep}copy-${_seq++}';
    try {
      await _fetch(v.files, stage);
      if (v.files.isEmpty) Directory(stage).createSync(recursive: true);
      _move(stage, _abs(e.kind, at));
      if (e.kind == LibraryKind.styles) {
        _fixStyleFile(_abs(e.kind, at), _stem(e.path));
      }
    } finally {
      _remove(stage);
    }
    _landed({e.kind});
    await syncNow();
  }

  Future<void> undelete(LibraryEntry e) async {
    await _quiet();
    final n = e.head?.n ?? _remote[e.key]?.n;
    if (n == null) return;
    await _restoreTo(e.key, n);
    await syncNow();
  }

  Future<void> deleteForever(LibraryEntry e) async {
    await _quiet();
    await account.call('DELETE',
        '/v1/library/items/${e.key}?base=${e.head?.n ?? _remote[e.key]?.n ?? 0}&forever=1');
    await syncNow();
  }

  /// Keep an item of the account on this computer from now on, though its
  /// kind is off here or it was removed from here.
  Future<void> get(LibraryEntry e) async {
    await _quiet();
    _got.add(e.key);
    _removed.remove(e.key);
    _save();
    _push('get', [e], () async => _removeHere(e), () => get(e));
    await syncNow();
  }

  Future<void> removeHere(LibraryEntry e) async {
    await _quiet();
    final back = _removeHere(e);
    _push('removeHere', [e], back, () => removeHere(e));
    await syncNow();
  }

  /// Stop keeping [e] here: to the trash, and never fetched again until Get.
  Future<void> Function() _removeHere(LibraryEntry e) {
    // Where it is now, which for an undone Get is where Get put it.
    final j = _items[e.key];
    final path = e.here ? e.path : j == null ? null : _renamed[e.key] ?? j.path;
    _removed.add(e.key);
    _got.remove(e.key);
    _items.remove(e.key);
    _renamed.remove(e.key);
    final kept = path == null ? null : _trash(e.kind, path);
    _save();
    _landed({e.kind});
    return () async {
      _removed.remove(e.key);
      if (kept != null) _putBack(e.kind, kept, path!);
      _save();
      _landed({e.kind});
    };
  }

  /// Never send [e] up again, and take it out of the account if it is
  /// there. The window asks first.
  Future<void> keepOnlyHere(LibraryEntry e) async {
    await _quiet();
    final n = _remote[e.key]?.n ?? e.head?.n;
    if (e._inAccount && n != null) {
      await account.call('DELETE', '/v1/library/items/${e.key}?base=$n');
      _items.remove(e.key);
      _renamed.remove(e.key);
    }
    _onlyHere.add((e.kind, e.path));
    _save();
    _push('keepOnlyHere', [e], () async {
      _onlyHere.remove((e.kind, e.path));
      if (e._inAccount && n != null) await _restoreTo(e.key, n);
    }, () => keepOnlyHere(e));
    await syncNow();
  }

  Future<void> shareAgain(LibraryEntry e) async {
    await _quiet();
    _onlyHere.remove((e.kind, e.path));
    _save();
    await syncNow();
  }

  Future<void> resolve(LibraryConflict c, LibraryResolution r) async {
    await _quiet();
    final id = c.entry.key, k = c.entry.kind, mine = c.entry.path;
    final held = _conflicts[id];
    if (held == null) return;
    final theirs = '${held['path'] ?? mine}';
    // Saved here since the last read: read again first, and the person sees
    // what they are choosing between before anything moves.
    for (final p in {mine, theirs}) {
      if (_exists(_abs(k, p)) && !_unchanged(k, p)) return syncNow();
    }
    void takeTheirs() {
      _trash(k, theirs);
      _move(c.theirsPath, _abs(k, theirs));
      _items[id] = LibraryItem(id, k, theirs, c.theirs.n, c.theirs.files);
      _renamed.remove(id);
    }

    final Future<void> Function() back;
    switch (r) {
      case LibraryResolution.keepMine:
        final files = _here[(k, _herePath(k, mine) ?? mine)]?.files;
        if (files == null) return;
        try {
          await _upload(k, id, mine, c.theirs.n, files);
        } on _Stale {
          return syncNow();
        }
        back = () => _restoreTo(id, c.theirs.n);
      case LibraryResolution.keepTheirs:
        final kept = _trash(k, mine);
        takeTheirs();
        back = () async {
          _trash(k, theirs);
          if (kept != null) _putBack(k, kept, mine);
        };
      case LibraryResolution.keepBoth:
        final named = keepBothName(k, mine, _thisName, _taken());
        var aside = await _renameItem(k, mine, _stem(named), keepId: false);
        if (aside == null) {
          // Moved as it is when the app's own rename refuses. When even that
          // fails, nothing is taken and the conflict waits.
          try {
            _move(_abs(k, mine), _abs(k, named));
            aside = named;
          } on FileSystemException {
            return;
          }
        }
        takeTheirs();
        back = () async {
          _trash(k, theirs);
          if (aside != null) {
            await _renameItem(k, aside, c.entry.name, keepId: false);
          }
        };
    }
    _conflicts.remove(id);
    _remove(File(c.theirsPath).parent.path);
    _save();
    _landed({k});
    _push('resolve', [c.entry], () async {
      await back();
      _landed({k});
    }, null);
    await syncNow();
  }

  /// Write [e], or everything here when it is empty, to one `.lumlibrary`.
  Future<int> exportTo(String file, List<LibraryEntry> e) => libraryPack(
        items: [
          for (final x in e.isEmpty ? _entries : e)
            if (x.pathHere case final at?)
              BridgeLibraryPackItem(kind: x.kind.name, path: at, name: x.path),
        ],
        out: file,
      );

  Future<int> importFrom(String file) async {
    final landed = <LibraryKey>{};
    await _unpack(file, landed);
    _landed({for (final k in landed) k.$1});
    await syncNow();
    return landed.length;
  }

  @override
  void dispose() {
    _gone = true;
    _settle?.cancel();
    _beat?.cancel();
    _later?.cancel();
    _unwatch();
    if (_started) {
      account.removeListener(_follow);
      profiles.removeListener(_follow);
    }
    _http.close(force: true);
    super.dispose();
  }
}
