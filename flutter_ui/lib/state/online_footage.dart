// The footage of a shared project that is kept online.
//
// Lumit's server keeps the footage as well as the project, sealed the same
// way, so nobody waits for someone else's machine to be on. The engine seals
// a file and opens one, and has no way to the server. This moves the sealed
// files there and back.
//
// What goes without being asked is a stand-in: up, of whatever this machine
// has and the server lacks, and down, of whatever this machine lacks. An
// original goes up only where the host has said to keep them online, and
// comes down only when someone asks for it.
//
// It follows the open project by itself, and runs only while that project is
// kept online and in step with the server. Nothing here is on a rebuild path.

import 'dart:async';
import 'dart:io';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/share.dart';

import 'account.dart';
import 'app_state.dart';
import 'transfer.dart';
import 'workspace.dart';

/// Where one file has got to.
enum MoveStage { waiting, working, failed }

/// One file on its way to Lumit's server or back, for the Shared project
/// window to list.
class OnlineMove {
  /// The footage item, what the server files it under, and its name.
  final FootageReference item;
  final String key, name;

  /// The file itself, and otherwise a stand-in of it.
  final bool original;

  /// This machine is sending it, and otherwise fetching it.
  final bool sending;

  MoveStage stage = MoveStage.waiting;

  /// Sealed bytes moved, of how many. Both 0 while a file is being made
  /// ready to send.
  int done = 0, total;

  /// Why it failed: the server's code, or one of this file's own.
  String? failure;

  HttpClient? _http;
  bool _cancelled = false, _tried = false;

  OnlineMove(this.item, this.key, this.name,
      {required this.original, required this.sending, this.total = 0});

  String get _kind => original ? 'original' : 'standin';
  String get _id => '$key/$_kind';
}

/// A move that stopped, and this file's own word for why.
class _Stopped implements Exception {
  final String code;
  const _Stopped(this.code);
}

/// How big a file of [bytes] is once it is sealed: a line saying what it is
/// and the file's own nonce, 23 bytes between them, then the file in pieces
/// of a mebibyte with a 16-byte seal on each. There is always one piece.
int sealedSize(int bytes) =>
    23 + bytes + 16 * max(1, (bytes + (1 << 20) - 1) >> 20);

/// How often a mover that stopped because the host's storage was full asks
/// whether there is room again.
const Duration _roomBeat = Duration(minutes: 5);

/// How often the server is asked what it holds.
// ponytail: asked on a timer, so footage someone else has just put online
// shows up within half a minute and not at once. A note from whoever stored
// it would be quicker, if the wait ever shows.
const Duration _beat = Duration(seconds: 30);

class OnlineFootage extends ChangeNotifier {
  final LumitState app;
  final AccountState account;
  final Workspace prefs;

  OnlineFootage(this.app, this.account, this.prefs) {
    app.share.roster.addListener(_follow);
    app.share.footage.addListener(look);
  }

  /// Every file moving, waiting to, or that failed, oldest first.
  final List<OnlineMove> moves = [];

  /// How big the originals are that this machine has, a composition uses and
  /// the server lacks: what keeping the originals online would send.
  int originalsBytes = 0;

  /// The server said the host's storage is full, so nothing more is sent
  /// until it says there is room again. It is asked every few minutes, and
  /// whenever [recheck] is.
  bool full = false;

  /// The next file there is to send, which is what the server is asked
  /// about while [full], and when it was last asked.
  ({BridgeOnlineFootage item, bool original})? _next;
  DateTime _roomAsked = DateTime(0);
  bool _asking = false;

  /// What the server held when it was last asked, by key and kind, each with
  /// its sealed size.
  Map<String, int> _held = const {};

  /// What the server files each footage item under, by the item's id.
  final Map<String, String> _keys = {};

  /// What the person asked for from a row's menu, and what they cancelled.
  final Set<String> _asked = {}, _left = {};

  /// A file two machines were giving at once is left alone until then.
  final Map<String, DateTime> _notBefore = {};

  ProjectReference? _project;
  String _state = '';
  Timer? _timer, _soon;
  Future<void>? _looking, _up, _down;
  bool _again = false, _gone = false;

  /// The server would store nothing more because the host's Lumit Pro has
  /// ended, as far as the last file sent could tell.
  bool _unpaid = false;

  /// Every seal and open in flight is stopped together, so each remembers
  /// how many stops there had been when it began.
  int _stops = 0;

  final Random _random = Random();

  BridgeShareOnline? get _online => _project == null ? null : app.share.online;

  void _tell() {
    if (!_gone) notifyListeners();
  }

  /// The project kept online opened, closed or changed, or it lost or found
  /// the server.
  void _follow() {
    if (_gone) return;
    final share = app.share;
    final online = share.online;
    final project = online == null ? null : app.project;
    if (project != _project) {
      _stop();
      _project = project;
      if (project != null) _timer = Timer.periodic(_beat, (_) => look());
    }
    final state =
        '${online?.id}|${online?.host}|${share.away}|${share.writable}';
    if (state == _state) return;
    _state = state;
    // The server may be taking files again.
    _unpaid = false;
    look();
  }

  void _stop() {
    _timer?.cancel();
    _soon?.cancel();
    _timer = _soon = null;
    if (_project == null) return;
    _project = null;
    for (final move in moves) {
      move._cancelled = true;
      move._http?.close(force: true);
    }
    _cancelPasses();
    moves.clear();
    _held = const {};
    _keys.clear();
    _asked.clear();
    _left.clear();
    _notBefore.clear();
    originalsBytes = 0;
    finished = 0;
    full = false;
    _next = null;
    _roomAsked = DateTime(0);
    _tell();
  }

  void _cancelPasses() {
    _stops++;
    try {
      onlineCancel();
    } catch (_) {
      // No engine to tell.
    }
  }

  /// How big the original of [item] is on the server, sealed, or null when
  /// nobody has put it online.
  int? originalOnline(FootageReference item) =>
      _held['${_keys['${item.internalid}']}/original'];

  /// Whether the server has a stand-in of [item].
  bool standInOnline(FootageReference item) =>
      _held.containsKey('${_keys['${item.internalid}']}/standin');

  /// Fetch [item] from the server whatever the settings say: a stand-in of
  /// it, or with [original] the file itself.
  void fetch(FootageReference item, {required bool original}) {
    final key = _keys['${item.internalid}'];
    if (key == null) return;
    final id = '$key/${original ? 'original' : 'standin'}';
    _asked.add(id);
    _left.remove(id);
    look();
  }

  /// How big the originals the server holds for this project are, sealed.
  int get originalsOnline => _held.entries
      .where((file) => file.key.endsWith('/original'))
      .fold(0, (sum, file) => sum + file.value);

  /// How many files have got there since the list was last empty, which
  /// with [moves] is how far along it all is.
  int finished = 0;

  /// Room may have been made in the host's storage: ask now. Harmless when
  /// nothing was stopped.
  void recheck() => unawaited(_askRoom());

  /// Ask the server to start the next file. It is told how big the sealed
  /// file would be, which is worked out from the file as it is, so nothing
  /// is sealed to find out. If it takes it there is room, and sending goes
  /// on as it does. If it does not, this stays stopped and says nothing new.
  Future<void> _askRoom() async {
    final project = _project;
    final online = _online;
    final next = _next;
    if (!full || _asking || project == null || online == null) return;
    // Nothing is left to send, so there is nothing to be full for.
    if (next == null) {
      full = false;
      return _tell();
    }
    _asking = true;
    _roomAsked = DateTime.now();
    try {
      final source = await next.item.item.onlineGive(original: next.original);
      if (source == null || project != _project) return;
      final kind = next.original ? 'original' : 'standin';
      await account.ask(
          'POST', '/v1/online/${online.id}/footage/${next.item.key}/$kind',
          body: {'size': sealedSize(await File(source).length())},
          headers: {'X-Lumit-Online': online.access});
      if (project != _project) return;
      full = false;
      _tell();
      look();
    } catch (_) {
      // Still full, out of reach, or closed. Asked again in a while.
    } finally {
      _asking = false;
    }
  }

  /// Try a [move] that failed again.
  void retry(OnlineMove move) {
    if (move.stage != MoveStage.failed) return;
    move
      ..stage = MoveStage.waiting
      ..failure = null
      ..done = 0
      .._tried = false;
    _tell();
    look();
  }

  /// Give up on [move]. It is not tried again until the project is next
  /// opened, unless it is asked for.
  void cancel(OnlineMove move) {
    move._cancelled = true;
    move._http?.close(force: true);
    _left.add(move._id);
    _asked.remove(move._id);
    // The engine stops every file it is sealing or opening, and the other
    // one in hand starts again by itself.
    if (move.stage == MoveStage.working) _cancelPasses();
    moves.remove(move);
    _tell();
  }

  /// Look at what there is to move, a moment from now: after a setting that
  /// decides it changed, or the project's footage did.
  void look() {
    if (_project == null || _gone || (_soon?.isActive ?? false)) return;
    _soon = Timer(const Duration(milliseconds: 400), () {
      if (_looking != null) {
        _again = true;
        return;
      }
      _looking = _look().whenComplete(() {
        _looking = null;
        if (_again) {
          _again = false;
          look();
        }
      });
    });
  }

  Future<void> _look() async {
    final project = _project;
    final online = _online;
    if (project == null || online == null || app.share.away) return;
    final Map<String, int> held;
    final List<BridgeOnlineFootage> items;
    try {
      final answer = await account.ask(
          'GET', '/v1/online/${online.id}/footage',
          headers: {'X-Lumit-Online': online.access});
      held = {
        if (answer['files'] case final List<dynamic> files)
          for (final file in files)
            if (file is Map && file['size'] is int)
              '${file['key']}/${file['kind']}': file['size'] as int,
      };
      if (project != _project) return;
      items = project.onlineFootage();
    } catch (_) {
      // Out of reach, or closed. Asked again on the next beat.
      return;
    }
    _held = held;
    _keys
      ..clear()
      ..addAll({for (final item in items) '${item.item.internalid}': item.key});

    // A machine that can only look fetches as anyone does and gives
    // nothing: the server would take none of it.
    final giving = prefs.shareGive &&
        app.share.writable &&
        !full &&
        !_unpaid &&
        !online.look;
    var keeping = false;
    try {
      keeping = online.host && prefs.shareOriginals.contains(project.shareId());
    } catch (_) {
      // Closed.
    }
    final now = DateTime.now();
    // What would be sent next were it not for [full].
    final sends =
        prefs.shareGive && app.share.writable && !_unpaid && !online.look;
    ({BridgeOnlineFootage item, bool original})? next;
    final wanted = <String, OnlineMove>{};
    void want(BridgeOnlineFootage item,
        {required bool original, required bool sending, int size = 0}) {
      final move = OnlineMove(item.item, item.key, item.name,
          original: original, sending: sending, total: size);
      if (_left.contains(move._id)) return;
      if (_notBefore[move._id]?.isAfter(now) ?? false) return;
      wanted[move._id] = move;
    }

    var bytes = 0;
    for (final item in items) {
      final standIn = held['${item.key}/standin'];
      final original = held['${item.key}/original'];
      final here = item.here == BridgeFootageHere.original;
      if (here && item.used) {
        if (original == null) bytes += item.bytes.toInt();
        if (sends && standIn == null) {
          next ??= (item: item, original: false);
        } else if (sends && keeping && original == null) {
          next ??= (item: item, original: true);
        }
        if (giving && standIn == null) {
          want(item, original: false, sending: true);
        }
        if (giving && keeping && original == null) {
          want(item, original: true, sending: true);
        }
      }
      if (item.here == BridgeFootageHere.missing && _placed(item)) continue;
      if (item.here == BridgeFootageHere.missing &&
          standIn != null &&
          (item.used && prefs.shareTake ||
              _asked.contains('${item.key}/standin'))) {
        want(item, original: false, sending: false, size: standIn);
      }
      if (!here &&
          original != null &&
          _asked.contains('${item.key}/original')) {
        want(item, original: true, sending: false, size: original);
      }
    }
    originalsBytes = bytes;
    _next = next;
    if (full && now.difference(_roomAsked) >= _roomBeat) unawaited(_askRoom());
    // What is no longer wanted and has not started goes. What is new joins
    // the end. A file that failed stays where it is, to be read.
    moves.removeWhere((move) =>
        move.stage == MoveStage.waiting && !wanted.containsKey(move._id));
    for (final move in moves) {
      if (move.stage == MoveStage.waiting) move._tried = false;
      wanted.remove(move._id);
    }
    if (moves.isEmpty) finished = 0;
    moves.addAll(wanted.values);
    _tell();
    // ponytail: one file up and one down at a time, which is what holds
    // each to its limit and keeps two encodes off the machine at once. A
    // shared allowance would let several go together.
    _up ??= _lane(true).whenComplete(() => _up = null);
    _down ??= _lane(false).whenComplete(() => _down = null);
  }

  /// Whether a file fetched for [item] before is still on this machine. The
  /// engine is then told to read it again. A document that has just come
  /// back from the server does not say where this machine keeps what it was
  /// sent, and without this every return would fetch it all again.
  bool _placed(BridgeOnlineFootage item) {
    for (final original in const [true, false]) {
      final room = item.item.onlineRoom(original: original);
      if (room != null && File(room).existsSync()) {
        item.item.onlineArrived(original: original);
        return true;
      }
    }
    return false;
  }

  Future<void> _lane(bool sending) async {
    while (true) {
      final project = _project;
      final move = moves
          .where((move) =>
              move.sending == sending &&
              move.stage == MoveStage.waiting &&
              !move._tried)
          .firstOrNull;
      if (project == null || move == null || app.share.away) return;
      move
        ..stage = MoveStage.working
        .._tried = true;
      _tell();
      Object? failed;
      try {
        await (sending ? _give(project, move) : _get(project, move));
      } catch (e) {
        failed = e;
      }
      move._http?.close(force: true);
      move._http = null;
      // The project closed under it, and took the list with it.
      if (project != _project) return;
      _settle(move, failed);
      _tell();
    }
  }

  /// What becomes of [move] now that it has ended, with [failed] when it
  /// did not get there.
  void _settle(OnlineMove move, Object? failed) {
    final code = switch (failed) {
      null => null,
      _Stopped(:final code) => code,
      CloudError(offline: true) => 'offline',
      CloudError(:final code) => code,
      _ => 'unknown',
    };
    if (code == null || move._cancelled) {
      moves.remove(move);
      if (code == null) {
        finished++;
        _asked.remove(move._id);
        // The server has it now, which the next look would say too.
        if (move.sending && move.total > 0) {
          _held = {..._held, move._id: move.total};
        }
      }
      return;
    }
    // The host's storage is full, or its Lumit Pro has ended. Nothing more
    // can be stored, so nothing more is tried.
    if (code == 'storage_full' || code == 'pro_required') {
      if (code == 'pro_required') {
        _unpaid = true;
      } else if (!full) {
        full = true;
        _roomAsked = DateTime.now();
        app.postNotice(
            (_online?.host ?? false)
                ? l10n.backupErrorStorageFull
                : l10n.shareOnlineFull,
            error: true);
      }
      moves.removeWhere((other) => other.sending);
      return;
    }
    // Two machines were giving the same file, and the server keeps only one
    // that came whole from one of them. The other is very likely there by
    // now or soon will be.
    // ponytail: left alone for a while, a different while on each machine,
    // so two that hold the same file stop getting in each other's way. The
    // server saying who is giving what would settle it at once.
    if (code == 'bad_parts' || code == 'bad_part' || code == 'bad_size') {
      _notBefore[move._id] = DateTime.now()
          .add(Duration(minutes: 5, seconds: _random.nextInt(600)));
      moves.remove(move);
      return;
    }
    // The line, the server having a bad moment, the project moving to a new
    // invite, or another file being cancelled: tried again on the next look.
    final later = code == 'offline' ||
        code == 'stopped' ||
        code == 'not_found' ||
        code == 'slow_down' ||
        (failed is CloudError && failed.status >= 500);
    if (later) {
      move
        ..stage = MoveStage.waiting
        ..done = 0
        .._tried = true;
      // Nothing was wrong with it, so it need not wait for the next beat.
      if (code == 'stopped') look();
      return;
    }
    move
      ..stage = MoveStage.failed
      ..failure = code;
  }

  /// One call about the project's footage, as anyone holding its invite.
  Future<Map<String, dynamic>> _ask(String method, OnlineMove move,
      [String rest = '', Object? body]) {
    final online = _online;
    if (online == null) throw const _Stopped('stopped');
    return account.ask(method,
        '/v1/online/${online.id}/footage/${move.key}/${move._kind}$rest',
        body: body, headers: {'X-Lumit-Online': online.access});
  }

  void _check(OnlineMove move) {
    if (move._cancelled) throw const _Stopped('cancelled');
  }

  /// A seal or an open that came back false: stopped along with another
  /// file's, which is no fault of this one, or truly failed.
  Never _refused(int stops, String code) =>
      throw _Stopped(stops == _stops ? code : 'stopped');

  /// Seal what this machine has of [move] and send it.
  Future<void> _give(ProjectReference project, OnlineMove move) async {
    final source = await move.item.onlineGive(original: move.original);
    _check(move);
    if (source == null) throw const _Stopped('nothing');
    final folder = await Directory.systemTemp.createTemp('lumit-online-');
    try {
      final sealed = File('${folder.path}${Platform.pathSeparator}sealed');
      final stops = _stops;
      if (!await project.onlineSeal(source: source, sealed: sealed.path)) {
        _check(move);
        _refused(stops, 'nothing');
      }
      _check(move);
      final size = await sealed.length();
      final begun = await _ask('POST', move, '', {'size': size});
      // Someone else got it there first.
      if (begun['have'] == true) return;
      move.total = size;
      final http = move._http = HttpClient()
        ..connectionTimeout = const Duration(seconds: 10)
        ..userAgent = 'Lumit';
      final parts = await sendParts(
        http,
        sealed,
        size: size,
        partSize: begun['part_size'] is int ? begun['part_size'] as int : 0,
        count: begun['parts'] is int ? begun['parts'] as int : 0,
        at: (part) {
          final online = _online;
          if (online == null) throw const _Stopped('stopped');
          return Uri.parse('$cloudUrl/v1/online/${online.id}/footage/'
              '${move.key}/${move._kind}/parts/$part');
        },
        headers: () async => {'X-Lumit-Online': _online?.access ?? ''},
        onProgress: (done) {
          move.done = done;
          _tell();
        },
        check: () => _check(move),
        limit: () => prefs.shareUpLimit * 1024,
      );
      await _ask('POST', move, '/finish', {'parts': parts});
    } finally {
      try {
        await folder.delete(recursive: true);
      } catch (_) {
        // Still held for a moment by whatever was reading it.
      }
    }
  }

  /// Fetch [move] and have the engine open it into the project.
  ///
  /// The sealed bytes come down into a file beside where the opened one
  /// goes, and are carried on from if they are cut off. The engine opens a
  /// whole file or none, so nothing half-written is ever where it reads.
  Future<void> _get(ProjectReference project, OnlineMove move) async {
    final room = move.item.onlineRoom(original: move.original);
    final online = _online;
    if (room == null || online == null) throw const _Stopped('stopped');
    final sealed = File('$room.sealed');
    await sealed.parent.create(recursive: true);
    if (!await sealed.exists()) {
      final part = File('${sealed.path}.part');
      var have = await part.exists() ? await part.length() : 0;
      if (have >= move.total) have = 0;
      final http = move._http = HttpClient()
        ..connectionTimeout = const Duration(seconds: 10)
        ..userAgent = 'Lumit';
      const patience = Duration(seconds: 30);
      final HttpClientResponse answer;
      try {
        final request = await http.getUrl(Uri.parse(
            '$cloudUrl/v1/online/${online.id}/footage/${move.key}/${move._kind}'));
        request.headers.set('X-Lumit-Online', online.access);
        if (have > 0) request.headers.set(HttpHeaders.rangeHeader, 'bytes=$have-');
        answer = await request.close().timeout(patience);
      } catch (_) {
        _check(move);
        throw const CloudError('offline');
      }
      if (answer.statusCode ~/ 100 != 2) {
        answer.listen(null).cancel().ignore();
        throw CloudError(
            answer.statusCode == 404 ? 'not_found' : 'unknown',
            answer.statusCode);
      }
      // Anything but the rest of it is the whole of it, from the start.
      if (answer.statusCode != HttpStatus.partialContent) {
        have = 0;
        if (await part.exists()) await part.delete();
      }
      move.done = have;
      var told = DateTime.now();
      final bytes = answer.timeout(patience).map((chunk) {
        move.done += chunk.length;
        final now = DateTime.now();
        if (now.difference(told).inMilliseconds >= 100) {
          told = now;
          _tell();
        }
        return chunk;
      });
      try {
        await writeChecked(paced(bytes, () => prefs.shareDownLimit * 1024),
            null, sealed.path,
            resume: true);
      } catch (_) {
        _check(move);
        rethrow;
      }
    }
    _check(move);
    final stops = _stops;
    final opened = await project.onlineOpen(sealed: sealed.path, out: room);
    // Stopped along with another file's, and still good for the next try.
    if (!opened && stops != _stops) {
      _check(move);
      throw const _Stopped('stopped');
    }
    try {
      await sealed.delete();
    } catch (_) {
      // Gone already.
    }
    if (!opened) throw const _Stopped('unreadable');
    move.item.onlineArrived(original: move.original);
  }

  @override
  void dispose() {
    _gone = true;
    app.share.roster.removeListener(_follow);
    app.share.footage.removeListener(look);
    _stop();
    super.dispose();
  }
}
