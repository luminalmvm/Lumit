// Backup: a copy of a project that Lumit Pro keeps each time it is saved.
//
// A save is sent to Lumit's server, which keeps it as a new version. A
// project's versions are its history, and any of them can be fetched on any
// machine signed in to the same account.
//
// Nothing here stands in the way of a save. [BackupState.saved] is told a
// file has been written and returns straight away. The file is read, hashed
// and sent afterwards, and a save made while one is being sent takes the
// place of whatever is waiting, since only the newest ever needs sending.
//
// Between saves the newest autosave of the open project is sent as a version
// too, marked as one. It is looked for once a minute, and the project file
// is left alone.
//
// The server knows a project by an id made here, which travels inside the
// project file with the interface's own record. What this machine knows
// about each id is kept in a small file beside the profile, never in the
// project.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart' show StringCharacters;
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/shell/about_window_frb.dart' show lumitVersion;
import 'package:lumit_flutter/shell/welcome_frb.dart' show projectDisplayName;
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart' show listAutosaves;
import 'package:uuid/uuid.dart';

import 'account.dart';
import 'app_state.dart';
import 'profiles.dart';
import 'timecode.dart';
import 'transfer.dart';
import 'updates.dart' show versionFromBootLine;
import 'workspace.dart';

/// The largest project file that is backed up, which is the server's own
/// limit. A packed project past this stays on the machine.
const int backupLargest = 95000000;

/// What the interface's record calls the id inside a project file.
const String _key = 'backup';

DateTime _at(Object? seconds) =>
    DateTime.fromMillisecondsSinceEpoch((seconds is int ? seconds : 0) * 1000);

/// One save of a project, as the server keeps it.
class BackupVersion {
  final int n;
  final String hash;
  final int size;

  /// The machine that saved it, the Lumit that wrote it, and the name the
  /// person gave it, which is empty until they do.
  final String device, app, note;

  /// Whether it has a picture.
  final bool thumb;

  /// Whether it was sent from an autosave between two saves, and not from
  /// a save.
  final bool auto;

  /// The version its machine last had when it made this one, which is what
  /// it was made from. Null for a project's first.
  final int? base;
  final DateTime created;

  const BackupVersion({
    required this.n,
    required this.hash,
    required this.size,
    required this.created,
    this.device = '',
    this.app = '',
    this.note = '',
    this.thumb = false,
    this.auto = false,
    this.base,
  });

  static BackupVersion? from(Object? j) =>
      j is Map && j['n'] is int && j['hash'] is String
          ? BackupVersion(
              n: j['n'] as int,
              hash: j['hash'] as String,
              size: j['size'] is int ? j['size'] as int : 0,
              created: _at(j['created']),
              device: '${j['device'] ?? ''}',
              app: '${j['app'] ?? ''}',
              note: '${j['note'] ?? ''}',
              thumb: j['thumb'] == true,
              auto: j['auto'] == true,
              base: j['base'] is int ? j['base'] as int : null,
            )
          : null;

  /// The same version once its picture has been sent.
  BackupVersion get pictured => BackupVersion(
      n: n,
      hash: hash,
      size: size,
      created: created,
      device: device,
      app: app,
      note: note,
      thumb: true,
      auto: auto,
      base: base);

  /// An autosave nobody has named, which the history shows more quietly.
  bool get quiet => auto && note.isEmpty;
}

/// A project the account has in backup.
class BackupProject {
  final String id, name;
  final DateTime updated;

  /// How many versions are kept, and what they come to.
  final int versions, bytes;

  /// The newest version.
  final BackupVersion? head;

  const BackupProject({
    required this.id,
    required this.name,
    required this.updated,
    this.versions = 0,
    this.bytes = 0,
    this.head,
  });

  static BackupProject? from(Object? j) => j is Map && j['id'] is String
      ? BackupProject(
          id: j['id'] as String,
          name: '${j['name'] ?? ''}',
          updated: _at(j['updated']),
          versions: j['versions'] is int ? j['versions'] as int : 0,
          bytes: j['bytes'] is int ? j['bytes'] as int : 0,
          head: BackupVersion.from(j['head']),
        )
      : null;
}

/// How the open project stands with its backup.
enum BackupStatus { off, backedUp, sending, waiting, failed, tooLarge }

/// One save waiting to be sent.
class _Save {
  /// The project's backup id, the file to send, the project's name, and
  /// the id of the document inside.
  final String id, path, named, doc;
  final ProjectReference project;

  /// The version whose bytes the file is, when it was written from one and
  /// not saved from the open project.
  final int? pictured;

  /// Asks the server even when the file is what this machine last sent.
  final bool sure;

  /// The file is an autosave from between two saves, and not the project's
  /// own file.
  final bool auto;
  const _Save(this.id, this.path, this.named, this.doc, this.project,
      {this.pictured, this.sure = false, this.auto = false});
}

bool _samePath(String a, String b) {
  String fold(String path) {
    final forward = path.replaceAll('\\', '/');
    return Platform.isWindows ? forward.toLowerCase() : forward;
  }

  return fold(a) == fold(b);
}

/// How big the file at [path] is and what it hashes to, worked out off the UI
/// thread.
// ponytail: the whole file is read in to hash it, which the size limit keeps
// affordable. Hash it a piece at a time if that limit ever goes up.
Future<(int, String)> _measure(String path) => Isolate.run(() {
      final bytes = File(path).readAsBytesSync();
      return (bytes.length, sha256.convert(bytes).toString());
    });

// --- What a picture says it shows --------------------------------------------

/// The text chunk a picture's caption is in, up to where its words start:
/// the kind of chunk, its key, and the empty fields an `iTXt` has before
/// words that are UTF-8.
final List<int> _captionHead = [...'iTXtLumit'.codeUnits, 0, 0, 0, 0, 0];

/// The most of a composition's name a picture carries, in bytes.
const int _longestCaptionName = 240;

/// The check a PNG keeps on each of its chunks.
int _crc32(List<int> bytes) {
  var crc = 0xffffffff;
  for (final byte in bytes) {
    crc ^= byte;
    for (var bit = 0; bit < 8; bit++) {
      crc = crc.isOdd ? (crc >> 1) ^ 0xedb88320 : crc >> 1;
    }
  }
  return crc ^ 0xffffffff;
}

/// [png] saying what it shows: the composition's [name] and the [timecode],
/// in a text chunk after its header. Every reader of a PNG steps over one,
/// so the pixels are the same, and the picture says so wherever it goes. A
/// version made from another's picture keeps the truth of it.
Uint8List stillCaptioned(Uint8List png, String name, String timecode) {
  // The header is the first chunk of any PNG and 13 bytes long. A file that
  // is not so is left as it is.
  if (png.length < 33 || ByteData.sublistView(png).getUint32(8) != 13) {
    return png;
  }
  // A long name is cut between characters, never through one.
  final kept = StringBuffer();
  var bytes = 0;
  for (final character in name.characters) {
    bytes += utf8.encode(character).length;
    if (bytes > _longestCaptionName) break;
    kept.write(character);
  }
  final chunk = [..._captionHead, ...utf8.encode('$kept\n$timecode')];
  final ends = ByteData(8)
    ..setUint32(0, chunk.length - 4)
    ..setUint32(4, _crc32(chunk));
  return (BytesBuilder(copy: false)
        ..add(Uint8List.sublistView(png, 0, 33))
        ..add(ends.buffer.asUint8List(0, 4))
        ..add(chunk)
        ..add(ends.buffer.asUint8List(4))
        ..add(Uint8List.sublistView(png, 33)))
      .takeBytes();
}

/// What a picture says it shows, as the composition's name and the timecode.
/// Null for one that does not say, which is any picture made before they
/// did.
///
/// The picture comes from outside, so each step is checked against its
/// length, and words that are not a name and a timecode are not a caption.
(String, String)? stillCaption(Uint8List png) {
  final data = ByteData.sublistView(png);
  final head = _captionHead.length;
  // Chunks follow the eight bytes a PNG opens with. Each is its length, four
  // letters, that many bytes and a check. The caption is before the pixels,
  // so the search ends where they start.
  for (var at = 8; at + 12 <= png.length;) {
    final length = data.getUint32(at);
    if (length > png.length - at - 12) return null;
    if (String.fromCharCodes(png, at + 4, at + 8) == 'IDAT') return null;
    if (length + 4 > head &&
        length < 512 &&
        listEquals(Uint8List.sublistView(png, at + 4, at + 4 + head),
            _captionHead)) {
      final said = utf8.decode(
          Uint8List.sublistView(png, at + 4 + head, at + 8 + length),
          allowMalformed: true);
      final cut = said.lastIndexOf('\n');
      if (cut < 0) return null;
      final timecode = said.substring(cut + 1);
      return RegExp(r'^\d{1,4}(:\d{1,4}){3}$').hasMatch(timecode)
          ? (said.substring(0, cut), timecode)
          : null;
    }
    at += 12 + length;
  }
  return null;
}

/// The open project's backup, the account's backed-up projects, and every
/// call made to the server about either.
class BackupState extends ChangeNotifier {
  final LumitState app;
  final AccountState account;
  final ProfilesState profiles;

  /// The composition a save's picture is taken of, and the frame.
  final (CompositionReference?, int) Function() looking;

  BackupState(this.app, this.account, this.profiles, {required this.looking}) {
    _load();
    _profile = profiles.current.id;
    _who = account.account?.id;
    _pro = account.pro;
    profiles.addListener(_follow);
    account.addListener(_follow);
    app.addListener(_opened);
    _opened();
  }

  /// A picture of [comp] at [frame] as a PNG, 960 on its long edge: the
  /// history draws it about 740 across, and a smaller one would be soft
  /// there. A field so a test with no graphics adapter can put its own
  /// here, as [Workspace.compThumbnailPng] is, and drawn the same way.
  static Future<Uint8List?> Function(CompositionReference comp, int frame)
      stillPng = (comp, frame) =>
          Workspace.compStillPng(comp, frame, maxEdge: 960);

  /// The largest picture the server takes. A frame of 960 by 540 can't come
  /// to this whatever is in it.
  static const int _largestStill = 3 << 20;

  final HttpClient _http = HttpClient()
    ..connectionTimeout = const Duration(seconds: 10)
    ..userAgent = 'Lumit';

  /// The Lumit that is running, cut to the twenty characters the server
  /// takes.
  late final String _version = () {
    try {
      final version = versionFromBootLine(lumitVersion()) ?? '';
      return version.length > 20 ? version.substring(0, 20) : version;
    } catch (_) {
      return '';
    }
  }();

  bool _gone = false;
  String? _profile, _who;
  bool _pro = false;

  // --- What this machine knows ---------------------------------------------

  /// Whether backup is on, the folder a fetched project was last put in, and
  /// for each backup id: the path it was last seen at here, the newest
  /// version number this machine has seen, the hash of what it last sent,
  /// and whether the project is kept out.
  Map<String, dynamic> _facts = {};

  File get _file => File('${Workspace.storeFile().parent.path}'
      '${Platform.pathSeparator}profiles${Platform.pathSeparator}'
      '${profiles.current.id}.backup.json');

  void _load() {
    try {
      final j = jsonDecode(_file.readAsStringSync());
      _facts = j is Map<String, dynamic> ? j : {};
    } catch (_) {
      // No file yet, or one that will not read.
      _facts = {};
    }
  }

  void _keep() {
    try {
      _file.parent.createSync(recursive: true);
      _file.writeAsStringSync(
          const JsonEncoder.withIndent('  ').convert(_facts));
    } catch (_) {
      // A disk that will not take it costs a request or two next time.
    }
  }

  Map<String, dynamic> get _known {
    final known = _facts['projects'];
    if (known is Map<String, dynamic>) return known;
    return _facts['projects'] = <String, dynamic>{};
  }

  Map<String, dynamic> _fact(String id) {
    final fact = _known[id];
    if (fact is Map<String, dynamic>) return fact;
    return _known[id] = <String, dynamic>{};
  }

  /// The id whose file on this machine is at [path].
  String? _idAt(String path) {
    for (final MapEntry(:key, :value) in _known.entries) {
      if (value is Map &&
          value['path'] is String &&
          _samePath(value['path'] as String, path)) {
        return key;
      }
    }
    return null;
  }

  /// The id the file at [path] goes by, given the one written [inFile].
  ///
  /// A path this machine already knows keeps its id. A file carrying the id
  /// of another file that is still where it was is a copy of it, and a copy
  /// starts a history of its own the next time it is saved.
  // ponytail: a copy is only told from its original on the machine that has
  // both. Copied to a machine that has never seen the original, the two share
  // one history until either is saved somewhere new.
  String? _idFor(String path, String? inFile) {
    final here = _idAt(path);
    if (here != null || inFile == null) return here;
    final known = _known[inFile];
    final seen = known is Map ? known['path'] : null;
    final copy =
        seen is String && !_samePath(seen, path) && File(seen).existsSync();
    return copy ? null : inFile;
  }

  static String? _idIn(String? record) {
    try {
      final j = record == null ? null : jsonDecode(record);
      final id = j is Map ? j[_key] : null;
      return id is String && RegExp(r'^[0-9a-f]{32}$').hasMatch(id) ? id : null;
    } catch (_) {
      return null;
    }
  }

  /// [id]'s file on this machine is at [path], and no other id's is.
  void _note(String id, String path) {
    final fact = _fact(id);
    // Nothing to write down on the save after the first, which is most.
    if (fact['path'] == path) return;
    for (final MapEntry(:key, :value) in _known.entries) {
      if (key != id &&
          value is Map &&
          value['path'] is String &&
          _samePath(value['path'] as String, path)) {
        value.remove('path');
      }
    }
    fact['path'] = path;
    _keep();
  }

  /// Where [id]'s file is on this machine, if it is still there.
  String? localPath(String id) {
    final known = _known[id];
    final path = known is Map ? known['path'] : null;
    return path is String && File(path).existsSync() ? path : null;
  }

  /// The folder a project fetched from backup is offered first: where the
  /// last one went, or Documents.
  String get folder {
    final last = _facts['folder'];
    if (last is String && Directory(last).existsSync()) return last;
    final home =
        Platform.environment['USERPROFILE'] ?? Platform.environment['HOME'];
    return '${home ?? '.'}${Platform.pathSeparator}Documents';
  }

  /// Whether projects are backed up when they are saved. On unless it has
  /// been switched off.
  bool get on => _facts['on'] != false;

  void setOn(bool value) {
    _facts['on'] = value;
    _keep();
    resend();
  }

  /// Whether the open project is kept out of backup.
  bool get out {
    final known = _known[id];
    return known is Map && known['out'] == true;
  }

  void setOut(bool value) {
    final path = _path;
    if (path == null) return;
    // A project with no id yet is given one, so there is something to file
    // the choice under. It reaches the file at the next save.
    final to = id ??= _mint();
    _note(to, path);
    _fact(to)['out'] = value;
    _keep();
    resend();
  }

  /// Whether a save of the open project is sent.
  bool get sends => account.pro && on && !out;

  // --- Following the profile and the account -------------------------------

  void _follow() {
    final profile = profiles.current.id;
    final who = account.account?.id;
    final pro = account.pro;
    if (profile == _profile && who == _who && pro == _pro) return;
    final switched = profile != _profile;
    // Pro has ended under a project that was being backed up. Said once,
    // here, however it came to be known.
    if (!switched && who == _who && !pro && id != null && on && !out) {
      app.postNotice(l10n.backupProEnded, error: true);
    }
    _profile = profile;
    _who = who;
    _pro = pro;
    // Read again, which also tells anything still under way that what it
    // was doing belongs to whoever was here before.
    _load();
    _waiting.clear();
    // What was seen and sent belongs to one account. A profile that has
    // only just been switched to still has the last one's account for a
    // moment, so that is not the time to ask.
    if (!switched && who != null && _facts['account'] != who) {
      for (final fact in _known.values) {
        if (fact is Map) fact..remove('seen')..remove('sent')..remove('large');
      }
      _facts['account'] = who;
      _keep();
    }
    projects = const [];
    used = limit = 0;
    listed = false;
    listError = null;
    // Pictures are kept by project and version number, and another account
    // can hold the same project with pictures of its own.
    _stills.clear();
    if (pro) unawaited(refresh());
    _look();
  }

  // --- The open project ----------------------------------------------------

  ProjectReference? _project;

  /// The open project's backup id, or null when it has none yet.
  String? id;

  BackupStatus status = BackupStatus.off;

  /// When what is on disk was backed up, where that is known.
  DateTime? backedUp;

  /// Why, when [status] is failed or waiting.
  CloudError? error;

  /// A version saved on another machine since this one last saw the project,
  /// when the file here is not it.
  BackupVersion? newer;

  /// Opens the Project history window. Null until there is a window to open
  /// it in.
  VoidCallback? showHistory;

  /// The id the document was last told to carry, until the save that wrote
  /// it has been heard of.
  String? _stamped;

  String? get _path {
    try {
      return _project?.path();
    } catch (_) {
      // Closed already.
      return null;
    }
  }

  /// The hash of the open project's file as this machine last sent it.
  String? get held {
    final known = _known[id];
    final sent = known is Map ? known['sent'] : null;
    return sent is String ? sent : null;
  }

  /// The hash of the open project's file as it is on disk now, which is
  /// not [held] once it has been saved again and that save has not gone.
  /// Null when there is no file to read.
  Future<String?> onDisk() async {
    final path = _path;
    if (path == null) return null;
    try {
      return (await _measure(path)).$2;
    } catch (_) {
      return null;
    }
  }

  static String _mint() => const Uuid().v4().replaceAll('-', '');

  void _say(String about, BackupStatus now, {CloudError? why}) {
    if (about != id || _gone) return;
    status = now;
    error = why;
    notifyListeners();
  }

  void _opened() {
    final project = app.project;
    if (identical(project, _project)) return;
    _project = project;
    _look();
  }

  /// Work out what the open project goes by, and ask how it stands.
  void _look() {
    final project = _project;
    id = _stamped = newer = backedUp = error = null;
    status = BackupStatus.off;
    final path = _path;
    try {
      if (path != null) id = _idFor(path, _idIn(project?.uiState()));
    } catch (_) {
      // Closed already.
    }
    final to = id;
    if (to != null && path != null) _note(to, path);
    _watch();
    notifyListeners();
    if (to != null && path != null && project != null && sends) {
      unawaited(_compare(to, path, project));
    }
  }

  /// Ask once how the file just opened stands against its history: the same
  /// as the newest version, behind one saved on another machine, or ahead
  /// with a save still to send.
  Future<void> _compare(
      String to, String path, ProjectReference project) async {
    final facts = _facts;
    try {
      final (_, hash) = await _measure(path);
      BackupVersion? head;
      var reached = true;
      try {
        head = (await history(to)).$1.head;
      } on CloudError catch (e) {
        // Not there is an answer. Anything else is no answer at all.
        reached = e.status == 404;
      }
      if (_gone || !identical(facts, _facts) || !identical(project, _project)) {
        return;
      }
      final fact = _fact(to);
      final seen = fact['seen'] is int ? fact['seen'] as int : 0;
      if (reached && head != null && head.hash == hash) {
        fact
          ..['seen'] = head.n
          ..['sent'] = hash;
        _keep();
        backedUp = head.created;
        _say(to, BackupStatus.backedUp);
      } else if (reached && head != null && head.n > seen) {
        newer = head;
        notifyListeners();
        app.postNotice(l10n.backupNewerNotice(head.device),
            onTap: () => showHistory?.call());
      } else if (!reached && fact['sent'] == hash) {
        _say(to, BackupStatus.backedUp);
      } else {
        // The newest version is not this file, and is nothing new to this
        // machine: an autosave it sent of work that was then never saved,
        // or a version deleted since. Either way the file here is sent
        // again, even if it is what was last sent.
        _queue(path, sure: reached);
      }
    } catch (_) {
      // The file went, or the project closed. Nothing to say about it.
    }
  }

  /// The interface's record as a save should write it: [record] with the
  /// project's backup id in it. Setting that record on the document is not
  /// an edit, so the id costs no undo step and no unsaved change, and it is
  /// written by the save that was happening anyway.
  ///
  /// [target] is where the save is going when that is somewhere new. A
  /// project saved somewhere new starts a history of its own, unless a file
  /// already there has one, which it carries on.
  String stamp(String record, String target) {
    final here = _path;
    final moving =
        target.isNotEmpty && (here == null || !_samePath(here, target));
    var to = moving ? _idAt(target) : id;
    if (to == null && account.pro && on) to = _mint();
    _stamped = to;
    if (to == null) return record;
    try {
      return jsonEncode(
          {...jsonDecode(record) as Map<String, dynamic>, _key: to});
    } catch (_) {
      return record;
    }
  }

  /// The project has just been saved to [path]. Returns at once.
  void saved(String path) {
    final was = id;
    final to = id = _stamped ?? id;
    _stamped = null;
    newer = null;
    if (to == null) return;
    // A project kept out of backup stays out when it is saved somewhere new.
    final kept = _known[was];
    if (was != to && kept is Map && kept['out'] == true) _fact(to)['out'] = true;
    _note(to, path);
    _watch();
    _queue(path);
  }

  /// Send the open project's file again, for a press that asks for it and
  /// for a switch that has just let it.
  void resend() {
    final path = _path;
    _watch();
    if (path != null) _queue(path);
    notifyListeners();
  }

  /// Put the open project's file in line to be sent, if it is one that is.
  void _queue(String path, {bool sure = false}) {
    final to = id, project = _project;
    if (to == null || project == null) return;
    if (!sends) {
      _waiting.remove(to);
      _say(to, BackupStatus.off);
      return;
    }
    _waiting[to] = _save(to, path, project, sure: sure);
    _say(to, BackupStatus.sending);
    unawaited(_pump());
  }

  _Save _save(String to, String path, ProjectReference project,
      {int? pictured, bool sure = false, bool auto = false}) {
    var doc = '';
    try {
      doc = project.shareId();
    } catch (_) {
      // Closed already. The id is for show.
    }
    // An autosave goes under the project's own name, not its file's.
    return _Save(to, path, _named(auto ? _path ?? path : path), doc, project,
        pictured: pictured, sure: sure, auto: auto);
  }

  /// One send at a time, this one waited for, with whatever was under way
  /// let finish first.
  Future<bool> _alone(_Save save) async {
    while (_pumping != null) {
      await _pumping;
    }
    if (!save.auto) _waiting.remove(save.id);
    final sending = _send(save);
    _pumping = sending;
    try {
      return await sending;
    } finally {
      _pumping = null;
      _later();
    }
  }

  /// The number of the version that is the open project's file as it is on
  /// disk, sending the file first when the server has no such version. Null
  /// when it could not be sent.
  ///
  /// This has to answer before that file is written over. A file the server
  /// already holds as a version makes no new one, so taking a newer version
  /// from another machine adds nothing to the history.
  Future<int?> versionOnDisk() async {
    final to = id, project = _project, path = _path;
    if (to == null || project == null || path == null || !on || out) {
      return null;
    }
    while (_pumping != null) {
      await _pumping;
    }
    try {
      final (_, hash) = await _measure(path);
      final (_, versions) = await history(to);
      final held = versions.where((v) => v.hash == hash).firstOrNull;
      if (held != null) return held.n;
    } catch (_) {
      // Not known. The send below asks, and says why when it can't.
    }
    // Without Pro the file can be looked for, and not sent.
    if (!sends || !await _alone(_save(to, path, project, sure: true))) {
      return null;
    }
    final seen = _fact(to)['seen'];
    return seen is int ? seen : null;
  }

  /// Record that the open project's file has just been written from
  /// [version], which makes that version the newest again. The server has
  /// the bytes already, so nothing is sent but the fact.
  Future<bool> recordWrittenFrom(BackupVersion version) async {
    final to = id, project = _project, path = _path;
    if (to == null || project == null || path == null || !sends) return false;
    return _alone(
        _save(to, path, project, pictured: version.n, sure: true));
  }

  // --- Autosaves between saves ----------------------------------------------

  /// How often the open project is looked at for an autosave worth sending.
  /// Lumit writes one beside the project every few minutes while there are
  /// unsaved changes.
  static const Duration _everySoOften = Duration(minutes: 1);
  Timer? _beat;

  /// Look for autosaves while there is an open project whose saves are sent,
  /// and not otherwise.
  void _watch() {
    final wanted = id != null && _path != null && sends && !_gone;
    if (!wanted) {
      _beat?.cancel();
      _beat = null;
    } else {
      _beat ??= Timer.periodic(_everySoOften, (_) => unawaited(checkpoint()));
    }
  }

  /// Send the newest autosave beside the open project as a version, when
  /// there is one that holds work no version has. True when one was sent.
  ///
  /// So a machine that dies takes a few minutes of work with it, and not
  /// everything since the last save. An autosave of a packed project is the
  /// document without its footage, so none is sent for one.
  Future<bool> checkpoint() async {
    final to = id, project = _project, path = _path;
    if (to == null || project == null || path == null || !sends) return false;
    if (_pumping != null) return false;
    try {
      if (!project.isDirty()) return false;
      final packs = project.packState();
      if (packs.autoPack || packs.packed > 0) return false;
      final newest = listAutosaves(project: path).firstOrNull;
      if (newest == null) return false;
      // One written before the last save holds nothing the save has not.
      final written = await File(newest.path).lastModified();
      if (!written.isAfter(await File(path).lastModified())) return false;
      if (_pumping != null || !identical(project, _project)) return false;
      return await _alone(_save(to, newest.path, project, auto: true));
    } catch (_) {
      // Closed, or the autosave went as it was looked at. Next time.
      return false;
    }
  }

  // --- Sending -------------------------------------------------------------

  /// The newest save of each project that has not gone yet.
  final Map<String, _Save> _waiting = {};
  Future<void>? _pumping;
  Timer? _retry;

  Future<void> _pump() => _pumping ??= _drain().whenComplete(() {
        _pumping = null;
        _later();
      });

  /// What could not go is tried again in a while, and at the next save.
  // ponytail: the wait is only kept while Lumit runs. After a restart a save
  // that never went is found again when its project is next opened, not
  // before.
  void _later() {
    _retry?.cancel();
    if (_waiting.isEmpty || _gone) return;
    _retry = Timer(const Duration(minutes: 2), _pump);
  }

  Future<void> _drain() async {
    while (_waiting.isNotEmpty && !_gone) {
      final save = _waiting.remove(_waiting.keys.first)!;
      if (!await _send(save)) break;
    }
  }

  /// Record one save with the server and send what it asks for. True once
  /// the server holds the file, and for an autosave only when it became a
  /// version.
  Future<bool> _send(_Save save) async {
    final facts = _facts;
    final fact = _fact(save.id);
    final about = '/v1/projects/${save.id}/versions';
    try {
      if (await File(save.path).length() > backupLargest) {
        throw const CloudError('too_large', 413);
      }
      final (size, hash) = await _measure(save.path);
      if (save.auto) {
        // The file as it was last saved, or the autosave last sent, over
        // again. Neither holds anything new.
        if (fact['auto'] == hash || fact['sent'] == hash) return false;
      } else if (!save.sure && fact['sent'] == hash) {
        _say(save.id, BackupStatus.backedUp);
        return true;
      }
      final answer = await account.call('POST', about, body: {
        'name': save.named,
        'doc': save.doc,
        'hash': hash,
        'size': size,
        'base': fact['seen'],
        'device': AccountState.deviceLabel,
        'app': _version,
        if (save.auto) 'auto': true,
      });
      final version = BackupVersion.from(answer['version']);
      if (version == null) throw const CloudError('unknown');
      var described = answer['project'];
      if (answer['upload'] == true) {
        final stored = await _exchange('PUT', '$about/${version.n}/file',
            send: File(save.path).openRead(), length: size);
        // The project with this version in it. The answer to the save was
        // given before the bytes arrived, so it is one version behind.
        try {
          final said = jsonDecode(await utf8
              .decodeStream(stored.timeout(const Duration(seconds: 30))));
          if (said is Map) described = said['project'];
        } catch (_) {
          // Stored all the same.
        }
      }
      // A version made from bytes the server already had comes with no
      // picture, so it is sent one as a new version is.
      var pictured = version.thumb;
      if (!version.thumb) {
        final png = await _picture(save);
        if (png != null && png.length <= _largestStill) {
          try {
            await _exchange('PUT', '$about/${version.n}/thumb',
                send: Stream.value(png),
                length: png.length,
                type: ContentType('image', 'png'));
            // Shown from here on, with no need to ask for it back.
            _stills['${save.id}/${version.n}'] = Future.value(png);
            pictured = true;
          } on CloudError {
            // A version with no picture is still a version.
          }
        }
      }
      if (_gone || !identical(facts, _facts)) return false;
      fact['seen'] = version.n;
      if (save.auto) {
        fact['auto'] = hash;
      } else {
        fact
          ..['sent'] = hash
          ..remove('large');
      }
      _keep();
      _listed(described, pictured: pictured);
      versionsSent++;
      if (save.auto) {
        // The file on disk is no more backed up than it was.
        notifyListeners();
        return true;
      }
      if (save.id == id) backedUp = version.created;
      _say(save.id, BackupStatus.backedUp);
      return true;
    } on CloudError catch (e) {
      if (_gone || !identical(facts, _facts)) return false;
      // An autosave that did not go is tried again when the next is looked
      // for. How the saved file stands is not its business.
      if (save.auto) return false;
      if (e.status == 402) {
        // Pro has gone, and the account is being asked again.
        _say(save.id, BackupStatus.off);
      } else if (e.status == 413) {
        if (fact['large'] != true) {
          fact['large'] = true;
          _keep();
          app.postNotice(l10n.backupTooLargeNotice(_named(save.path)));
        }
        _say(save.id, BackupStatus.tooLarge);
      } else {
        _waiting.putIfAbsent(save.id, () => save);
        _say(save.id,
            e.offline ? BackupStatus.waiting : BackupStatus.failed,
            why: e);
      }
      return false;
    } catch (_) {
      // The file has gone from where it was saved. Nothing to send.
      if (!save.auto) _say(save.id, BackupStatus.off);
      return false;
    }
  }

  /// How many versions have been sent since Lumit started, for a window
  /// that lists them to know when to ask again.
  int versionsSent = 0;

  static String _named(String path) {
    final name = projectDisplayName(path);
    return name.length > 120 ? name.substring(0, 120) : name;
  }

  /// The picture that goes with a save: the open project as it looks, while
  /// the project it was a save of is still the one open, saying which
  /// composition it is and where in it. A file that was written from an
  /// earlier version takes that version's picture, and what that one says.
  Future<Uint8List?> _picture(_Save save) async {
    if (save.pictured case final n?) return still(save.id, n);
    if (!identical(save.project, _project)) return null;
    try {
      final (comp, frame) = looking();
      if (comp == null) return null;
      // Read before the picture is drawn, so they are of the same moment.
      final shown = comp.getSettings();
      final png = await stillPng(comp, frame);
      return png == null
          ? null
          : stillCaptioned(png, shown.name,
              timecodeOfRate(frame, shown.fpsNum, shown.fpsDen));
    } catch (_) {
      // No graphics adapter, or a composition that will not draw.
      return null;
    }
  }

  /// Bytes to or from the server as the signed-in account. Throws a
  /// [CloudError] for anything but a 2xx answer.
  Future<HttpClientResponse> _exchange(String method, String path,
      {Stream<List<int>>? send, int length = 0, ContentType? type}) async {
    final token = await account.accessToken();
    if (token == null) throw const CloudError('signed_out', 401);
    final HttpClientResponse response;
    try {
      final request = await _http
          .openUrl(method, Uri.parse('$cloudUrl$path'))
          .timeout(const Duration(seconds: 15));
      request.headers.set(HttpHeaders.authorizationHeader, 'Bearer $token');
      Future<HttpClientResponse> exchange() async {
        if (send != null) {
          request.headers.contentType = type ?? ContentType.binary;
          request.contentLength = length;
          await request.addStream(send);
        }
        return request.close();
      }

      // A minute, and longer for a file: as long as it would take on a line
      // that manages sixteen kilobytes a second.
      response = await exchange().timeout(
          Duration(seconds: 60 + length ~/ 16384),
          onTimeout: () {
        request.abort();
        throw const CloudError('offline');
      });
    } catch (_) {
      throw const CloudError('offline');
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

  // --- The account's projects ----------------------------------------------

  /// The account's backed-up projects, most recently saved first, as the
  /// server last listed them.
  List<BackupProject> projects = const [];

  /// How much of the account's storage is used, and how much it has.
  int used = 0, limit = 0;

  /// How much of what is used is these backups. Review links fill the same
  /// storage.
  int get stored => projects.fold(0, (sum, project) => sum + project.bytes);

  /// Whether the server has answered with the list, and why not when the
  /// last try failed.
  bool listed = false;
  CloudError? listError;

  Future<void> refresh() async {
    final facts = _facts;
    try {
      final answer = await account.call('GET', '/v1/projects');
      if (_gone || !identical(facts, _facts)) return;
      projects = [
        for (final entry in answer['projects'] as List<dynamic>? ?? const [])
          if (BackupProject.from(entry) case final project?)
            // One whose first save never arrived has nothing to open.
            if (project.head != null) project,
      ];
      final usage = answer['usage'];
      if (usage is Map) {
        used = usage['bytes'] is int ? usage['bytes'] as int : 0;
        limit = usage['limit'] is int ? usage['limit'] as int : 0;
      }
      listed = true;
      listError = null;
    } on CloudError catch (e) {
      if (_gone || !identical(facts, _facts)) return;
      listError = e;
    }
    notifyListeners();
  }

  /// A project the server has just described goes to the top of the list.
  /// With [pictured] its newest version has had its picture sent since, so
  /// the list shows it now and not the next time it is asked for.
  void _listed(Object? described, {bool pictured = false}) {
    final said = BackupProject.from(described);
    final head = said?.head;
    if (said == null || head == null) return;
    projects = [
      BackupProject(
          id: said.id,
          name: said.name,
          updated: said.updated,
          versions: said.versions,
          bytes: said.bytes,
          head: pictured ? head.pictured : head),
      for (final other in projects)
        if (other.id != said.id) other,
    ];
  }

  /// A project and its versions, newest first.
  Future<(BackupProject, List<BackupVersion>)> history(String id) async {
    final answer = await account.call('GET', '/v1/projects/$id');
    final project = BackupProject.from(answer['project']);
    if (project == null) throw const CloudError('unknown');
    return (
      project,
      [
        for (final entry in answer['versions'] as List<dynamic>? ?? const [])
          if (BackupVersion.from(entry) case final version?) version,
      ],
    );
  }

  /// Fetch [version] of [id] to [path]. With [home] that file is the
  /// project's own on this machine from then on, and without it is a copy.
  Future<void> fetch(String id, BackupVersion version, String path,
      {bool home = false}) async {
    final response = await _exchange(
        'GET', '/v1/projects/$id/versions/${version.n}/file');
    await writeChecked(
        response.timeout(const Duration(seconds: 30)), version.hash, path);
    if (!home) return;
    _note(id, path);
    // Brought here from backup, so it is in backup, whatever was chosen for
    // a file of the same id that used to be here.
    _fact(id)
      ..['seen'] = version.n
      ..['sent'] = version.hash
      ..remove('out');
    _facts['folder'] = File(path).parent.path;
    _keep();
  }

  /// The pictures looked at last, newest last. A picture can be a megabyte,
  /// so a long history looked through end to end does not hold them all.
  final Map<String, Future<Uint8List?>> _stills = {};
  static const int _stillsKept = 48;

  /// A version's picture, or null when it has none. A version's picture
  /// never changes, so one that is still held is not fetched again.
  Future<Uint8List?> still(String id, int n) {
    final key = '$id/$n';
    final picture = _stills.remove(key) ?? _still(id, n);
    _stills[key] = picture;
    if (_stills.length > _stillsKept) _stills.remove(_stills.keys.first);
    return picture;
  }

  Future<Uint8List?> _still(String id, int n) async {
    try {
      final response =
          await _exchange('GET', '/v1/projects/$id/versions/$n/thumb');
      final bytes = BytesBuilder(copy: false);
      await response.forEach(bytes.add);
      return bytes.takeBytes();
    } on CloudError catch (e) {
      // Not reached is worth asking again. Not there is not.
      if (e.status != 404) unawaited(_stills.remove('$id/$n'));
      return null;
    }
  }

  /// Give a version a name, which also keeps it for good. Empty takes the
  /// name away.
  Future<void> nameVersion(String id, int n, String note) => account
      .call('PATCH', '/v1/projects/$id/versions/$n', body: {'note': note});

  /// Delete one version. When it was the open project's file as last sent,
  /// that file is no longer backed up, and the state says so until the next
  /// save or a press sends it again.
  Future<void> deleteVersion(String id, BackupVersion version) async {
    await account.call('DELETE', '/v1/projects/$id/versions/${version.n}');
    unawaited(_stills.remove('$id/${version.n}'));
    final fact = _fact(id);
    if (fact['auto'] == version.hash) fact.remove('auto');
    if (fact['sent'] == version.hash) {
      fact.remove('sent');
      if (id == this.id) backedUp = null;
      _say(id, BackupStatus.waiting);
    }
    _keep();
  }

  /// The bytes of one version, for reading what is in it. Not checked
  /// against its hash here: whoever reads them does that.
  Future<Uint8List> bytes(String id, BackupVersion version) async {
    final response = await _exchange(
        'GET', '/v1/projects/$id/versions/${version.n}/file');
    final bytes = BytesBuilder(copy: false);
    await response.timeout(const Duration(seconds: 30)).forEach(bytes.add);
    return bytes.takeBytes();
  }

  Future<void> rename(String id, String name) async {
    await account.call('PATCH', '/v1/projects/$id', body: {'name': name});
    await refresh();
  }

  /// Delete a project and its history from the server. Its file on this
  /// machine stays, and is kept out of backup from then on so that the next
  /// save does not put it straight back.
  Future<void> delete(String id) async {
    await account.call('DELETE', '/v1/projects/$id');
    _fact(id)
      ..['out'] = true
      ..remove('seen')
      ..remove('sent');
    _keep();
    _waiting.remove(id);
    _say(id, BackupStatus.off);
    await refresh();
  }

  @override
  void dispose() {
    _gone = true;
    _retry?.cancel();
    _beat?.cancel();
    profiles.removeListener(_follow);
    account.removeListener(_follow);
    app.removeListener(_opened);
    _http.close(force: true);
    super.dispose();
  }
}
