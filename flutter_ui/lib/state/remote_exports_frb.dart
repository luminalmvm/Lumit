// Exports one of the account's computers does for another (docs/01-GLOSSARY.md:
// Remote export), as far as the interface goes.
//
// The engine holds the jobs, decides which computer can take one, and keeps
// both ends going. This is the pipe between it and the account's room on
// Lumit's server: what this computer adds to what it says there, what the
// others ask of it and answer, sealed so the server can't read them, and the
// notices that say how a job ended. It rides the phones' line ([PhoneLine]),
// which is the one socket to the room.

import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart'
    show workAreaFrames;
import 'package:lumit_flutter/shell/export_dialog_frb.dart'
    show exportDefaultPath;
import 'package:lumit_flutter/shell/pro_window.dart' show writtenDay;
import 'package:lumit_flutter/shell/project_history_window.dart'
    show backupTime;
import 'package:lumit_flutter/shell/status_line_frb.dart'
    show statusLineExportStarted;
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:lumit_flutter/src/rust/api/remote.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart' show revealInFolder;

import 'account.dart';
import 'app_state.dart';
import 'file_dialogs.dart' show pickExportSaveLocation;
import 'friend_invites.dart';
import 'phone_line_frb.dart';
import 'preset_export.dart';
import 'relay_door.dart';
import 'workspace.dart';

/// What a box between two of the account's computers is sealed as, the
/// HKDF info both sides use, so it never opens as anything else.
const String computerBoxInfo = 'lumit computer v1';

/// Whether a job has stopped, one way or the other.
bool remoteEnded(BridgeRemoteStage stage) =>
    stage == BridgeRemoteStage.done ||
    stage == BridgeRemoteStage.failed ||
    stage == BridgeRemoteStage.cancelled;

/// Why a job stopped, in words: a code the engine or the room gave, or the
/// export's own words as they came. `not_found` is a kept file that has
/// gone from the other computer, the one thing Bring here can't fetch.
String remoteWhy(String error) => switch (error) {
      'too_old' => l10n.remoteWhyTooOld,
      'no_space' => l10n.remoteWhyNoSpace,
      'lost' => l10n.remoteWhyLost,
      'not_found' => l10n.remoteWhyGone,
      'turned_off' => l10n.remoteNotTaking,
      'cancelled' => l10n.remoteCancelled,
      // Any other code says only that it failed: `failed`, or the room's
      // own, such as `bad_request`.
      _ when _code.hasMatch(error) => l10n.remoteWhyFailed,
      _ => error,
    };

final RegExp _code = RegExp(r'^[a-z_]+$');

/// A computer that is offline or asleep, with when it was last online
/// ([seen], unix seconds, zero for never): the time today, the day before.
String remoteAwayWords(int seen) {
  if (seen <= 0) return l10n.remoteAway;
  final at = DateTime.fromMillisecondsSinceEpoch(seen * 1000);
  final now = DateTime.now();
  final today =
      at.year == now.year && at.month == now.month && at.day == now.day;
  return l10n.remoteAwaySince(today ? backupTime(at) : writtenDay(at));
}

/// How far a transfer has got, in the unit its whole size is written in:
/// 2.1 of 8.4 GB.
String remoteSizeOf(int done, int total) {
  final (step, unit) = total >= 1000000000
      ? (1e9, l10n.unitGb)
      : total >= 1000000
          ? (1e6, l10n.unitMb)
          : (1e3, l10n.unitKb);
  String amount(int bytes) {
    final n = bytes / step;
    final written = n.toStringAsFixed(n < 10 ? 1 : 0);
    return written.endsWith('.0')
        ? written.substring(0, written.length - 2)
        : written;
  }

  return l10n.remoteSizeOf(amount(done), '${amount(total)} $unit');
}

/// Bring the file of an export that stayed on the other computer here, to a
/// place picked with the save dialogue.
Future<void> remoteBringHere(LumitState app, String job, String file) async {
  final dot = file.lastIndexOf('.');
  final extension = dot > 0 ? file.substring(dot + 1) : '';
  final path = await pickExportSaveLocation(file,
      extension: extension, label: extension.toUpperCase());
  if (path == null) return;
  try {
    remoteFetch(job: job, path: path);
  } catch (_) {
    app.postNotice(l10n.remoteFetchFailed(file), error: true);
  }
}

class RemoteExports extends ChangeNotifier {
  final AccountState account;
  final LumitState app;
  final Workspace workspace;
  final PhoneLine phones;
  final FriendInvites friendInvites;
  final RelayDoor relayDoor;

  RemoteExports(this.account, this.app, this.workspace, this.phones,
      this.friendInvites, this.relayDoor);

  /// This computer's id in the room, as the last list of computers said.
  String? _me;

  /// This computer's public key, in hex, once there is one.
  String? _key;

  /// The account the engine was last told of.
  String? _for;

  bool _started = false;
  StreamSubscription<BridgeRemoteSay>? _outbox;
  StreamSubscription<BridgeExportNews>? _news;

  /// What is being sealed and said, one at a time so it goes in order.
  Future<void> _saying = Future.value();

  /// Begin. Not called by a test, which keeps a test run off the network.
  void start() {
    if (_started) return;
    _started = true;
    phones
      ..sayAlso = _sayAlso
      ..itemAlso = _itemAlso
      ..queueAlso = _queueAlso
      ..heardAlso = _heard
      ..queueOn = _queueOn;
    remoteSetTakes(on_: workspace.remoteTakes);
    remoteSetFolders(folders: workspace.remoteFolders);
    _outbox = remoteOutbox().listen(
        (say) => _saying = _saying.then((_) => _say(say)),
        onError: (_) {});
    _news = exportNews().listen(_told, onError: (_) {});
    friendInvites.addListener(_keysChanged);
    account.addListener(_accountChanged);
    _keysChanged();
    _accountChanged();
  }

  /// Take exports from the account's other computers, or stop.
  void setTakes(bool on) {
    workspace.setRemoteTakes(on);
    remoteSetTakes(on_: on);
    phones.sayNow();
  }

  /// The folders looked in for footage another computer sends.
  void setFolders(List<String> folders) {
    workspace.setRemoteFolders(folders);
    remoteSetFolders(folders: workspace.remoteFolders);
  }

  /// How much the files kept for the account's other computers take, and
  /// deleting them, but for those a job needs now.
  int keptBytes() => remoteKeptBytes();
  void clearKept() => remoteKeptClear();

  void _accountChanged() {
    final who = account.pro ? account.account?.id : null;
    if (who == _for) return;
    _for = who;
    _me = null;
    remoteAccount(account: who);
    // A job's files go through Lumit's relay when the two computers can't
    // reach each other.
    if (who != null) unawaited(relayDoor.open());
    notifyListeners();
  }

  Future<void> _keysChanged() async {
    final keys = friendInvites.keys;
    final key =
        keys == null ? null : hexOf((await keys.extractPublicKey()).bytes);
    if (keys == friendInvites.keys) _key = key;
  }

  // ---- what this computer says in the room ---------------------------------

  Map<String, Object?> _sayAlso() {
    Object? fields;
    try {
      fields = jsonDecode(remoteStateFields());
    } catch (_) {
      fields = null;
    }
    return {
      if (fields is Map<String, dynamic>) ...fields,
      'takes': workspace.remoteTakes,
      if (_key != null) 'key': _key,
    };
  }

  /// The computer the file of [job] ends up on.
  String? _lands(BridgeRemoteJob job) =>
      (job.lands == BridgeRemoteLands.sender) == job.sent ? _me : job.other;

  /// The room's `state` and `stage` for [job]: `state` is one of the four a
  /// queue item has, and `stage` the finer step a job has reached.
  static (String, String?) _room(BridgeRemoteJob job) => switch (job.stage) {
        BridgeRemoteStage.away => ('waiting', 'away'),
        BridgeRemoteStage.checking => ('waiting', 'checking'),
        BridgeRemoteStage.sending => ('waiting', 'sending'),
        BridgeRemoteStage.queued => ('waiting', 'queued'),
        // Held where it was: written, under way, or not yet begun.
        BridgeRemoteStage.paused => (
            job.total > 0 && job.frame >= job.total
                ? 'done'
                : job.frame > 0
                    ? 'running'
                    : 'waiting',
            'paused'
          ),
        BridgeRemoteStage.exporting => ('running', null),
        BridgeRemoteStage.returning => ('done', 'returning'),
        BridgeRemoteStage.done => (
            'done',
            job.lands == BridgeRemoteLands.worker ? 'kept' : 'landed'
          ),
        BridgeRemoteStage.failed || BridgeRemoteStage.cancelled => ('failed', null),
      };

  /// A job this computer does for another, on its item in the queue.
  Map<String, Object?> _itemAlso(BridgeExportQueueItem item) {
    if (item.job.isEmpty) return const {};
    final job = remoteJobs().where((j) => j.job == item.job).firstOrNull;
    if (job == null) return {'job': item.job};
    final (_, stage) = _room(job);
    return {
      'job': job.job,
      'from': job.other,
      'lands': _lands(job),
      if (stage != null) 'stage': stage,
      'bytes': job.bytes,
      'size': job.size,
    };
  }

  /// A mirror of each job this computer sent, beside its own items.
  List<Map<String, Object?>> _queueAlso() => [
        for (final job in remoteJobs())
          if (job.sent) _mirror(job),
      ];

  Map<String, Object?> _mirror(BridgeRemoteJob job) {
    final (state, stage) = _room(job);
    return {
      'id': job.item,
      'name': job.compName,
      'file': job.file,
      'preset': job.preset,
      'codec': job.codec,
      'state': state,
      'frame': job.frame,
      'total': job.total,
      'error': job.stage == BridgeRemoteStage.cancelled
          ? 'cancelled'
          : state == 'failed'
              ? job.error
              : null,
      'started': null,
      'ended': job.ended > 0 ? job.ended : null,
      'job': job.job,
      'on': job.other,
      'lands': _lands(job),
      if (stage != null) 'stage': stage,
      'bytes': job.bytes,
      'size': job.size,
    };
  }

  /// Seal and say what the engine has to say.
  Future<void> _say(BridgeRemoteSay say) async {
    try {
      String? box;
      if (say.body.isNotEmpty) {
        // Nothing to seal to yet. The engine says a command again until it
        // is answered.
        if (say.key.isEmpty) return;
        box = await sealBox(utf8.encode(say.body), unhex(say.key),
            info: computerBoxInfo);
      }
      phones.send(say.reply
          ? {
              'type': 'reply',
              'id': say.id,
              'to': say.to,
              if (say.error.isNotEmpty)
                'error': say.error
              else ...{'ok': true, if (box != null) 'box': box},
            }
          : {
              'type': 'command',
              'id': say.id,
              'computer': say.to,
              'do': 'job',
              if (box != null) 'box': box,
            });
    } catch (_) {
      // A key that isn't one. Said again by the engine, or given up there.
    }
  }

  // ---- what this computer hears --------------------------------------------

  void _heard(Map<dynamic, dynamic> j) {
    switch (j['type']) {
      case 'computers':
        final all = j['computers'];
        if (all is List) {
          for (final c in all) {
            if (c is Map && c['me'] == true && c['id'] is String) {
              _me = c['id'] as String;
            }
          }
        }
        remoteHeard(said: jsonEncode(j));
        notifyListeners();
      case 'reply':
        unawaited(_reply(j));
      case 'command' when j['do'] == 'job':
        unawaited(_command(j));
      case 'command':
        _mirrorCommand(j);
    }
  }

  /// [sealed] opened with this computer's key, or null.
  Future<String?> _open(Object? sealed) async {
    final keys = friendInvites.keys;
    if (sealed is! String || keys == null) return null;
    final opened = await openBox(sealed, keys, info: computerBoxInfo);
    if (opened == null) return null;
    try {
      return utf8.decode(opened);
    } catch (_) {
      return null;
    }
  }

  Future<void> _command(Map<dynamic, dynamic> j) async {
    final id = '${j['id']}', from = '${j['from']}';
    final body = await _open(j['box']);
    if (body == null) {
      phones.send(
          {'type': 'reply', 'id': id, 'to': from, 'error': 'bad_request'});
      return;
    }
    remoteCommand(id: id, from: from, body: body);
  }

  Future<void> _reply(Map<dynamic, dynamic> j) async {
    final id = '${j['id']}';
    final error = j['error'] is String ? j['error'] as String : '';
    if (j['box'] == null) {
      remoteReply(id: id, body: '', error: error);
      return;
    }
    final body = await _open(j['box']);
    remoteReply(
        id: id,
        body: body ?? '',
        error: body == null && error.isEmpty ? 'bad_request' : error);
  }

  /// A phone's cancel, remove or preview on a job this computer sent, which
  /// the computer doing it is told of by the engine.
  void _mirrorCommand(Map<dynamic, dynamic> j) {
    final job = remoteJobs()
        .where((job) => job.sent && job.item == j['item'])
        .firstOrNull;
    String? error;
    if (!workspace.phonesExport) {
      error = 'turned_off';
    } else if (job == null) {
      error = 'not_found';
    } else {
      switch (j['do']) {
        case 'cancel':
          remoteCancel(job: job.job);
          app.postNotice(l10n.phoneCancelledExport(job.compName));
        case 'remove':
          remoteRemove(job: job.job);
          app.postNotice(l10n.phoneRemovedExport(job.compName));
        case 'again':
          try {
            remoteAgain(job: job.job);
          } catch (_) {
            error = 'failed';
          }
        default:
          // ponytail: a preview of a job comes from the computer that did it,
          // which the phone asks itself.
          error = 'failed';
      }
    }
    phones.send({
      'type': 'reply',
      'id': j['id'],
      'to': j['from'],
      if (error == null) 'ok': true else 'error': error,
    });
    phones.sayNow();
  }

  /// A phone asking this computer, which has the project open, to send an
  /// export to another of the account's computers: why not, or null once it
  /// has gone there or is held for it.
  Future<String?> _queueOn(Map<dynamic, dynamic> j) async {
    final project = app.project;
    if (project == null) return 'no_project';
    final on = j['on'] as String;
    if (on == j['computer'] || on == _me) return 'same';
    final comp = app
        .comps()
        .where((c) => '${c.$1.internalid}' == j['composition'])
        .firstOrNull
        ?.$1;
    final preset = j['preset'];
    if (comp == null || preset is! String || preset.isEmpty) return 'not_found';
    final stored = exportPresetGet(name: preset);
    if (stored == null) return 'not_found';
    final computer = remoteComputers().where((c) => c.id == on).firstOrNull;
    if (computer == null) return 'not_found';
    if (!computer.takes) return 'turned_off';
    if (comp.remoteFit(computer: on) is BridgeRemoteFit_TooOld) {
      return 'too_old';
    }
    final lands = j['lands'] == j['computer']
        ? BridgeRemoteLands.sender
        : BridgeRemoteLands.worker;
    final settings = comp.getSettings();
    // Where Settings > Export says, as an export from a phone done here goes.
    // A file that stays there needs only its name.
    var path = exportDefaultPath(exportDefaultsGet(),
            preset: preset,
            compName: settings.name,
            codec: stored.codec,
            projectPath: project.path()) ??
        (lands == BridgeRemoteLands.worker
            ? exportPreset(
                    preset: preset,
                    compName: settings.name,
                    template: exportDefaultsGet().filenameTemplate)
                .defaultName
            : null);
    if (path == null || path.isEmpty) return 'no_folder';
    // Nobody at a phone was asked about a file already there, so it is kept,
    // as with an export from a phone done here.
    if (lands == BridgeRemoteLands.sender &&
        exportPathTaken(
            path: path, codec: stored.codec, captions: stored.captions)) {
      path = exportFreePath(
          path: path, codec: stored.codec, captions: stored.captions);
    }
    final area = j['range'] == 'whole' ? null : workAreaFrames(comp);
    try {
      comp.remoteSend(
          computer: on,
          spec: presetExportSpec(
              preset: preset,
              stored: stored,
              settings: settings,
              start: area?.start ?? 0,
              end: area?.end ?? comp.durationFrames()),
          path: path,
          lands: lands);
    } catch (_) {
      return 'failed';
    }
    // Wake the status line, which follows a sent export as it does one here.
    statusLineExportStarted.value++;
    app.postNotice(l10n.remotePhoneSent(settings.name, computer.name));
    return null;
  }

  // ---- how exports end -----------------------------------------------------

  void _told(BridgeExportNews news) {
    switch (news.kind) {
      case BridgeExportNewsKind.landed:
        app.postNotice(l10n.remoteNewsLanded(news.file, news.otherName),
            action: news.path.isEmpty ? null : l10n.remoteShow,
            onTap: news.path.isEmpty
                ? null
                : () => revealInFolder(path: news.path));
      case BridgeExportNewsKind.kept:
        app.postNotice(l10n.remoteNewsKept(news.otherName, news.file),
            action: l10n.remoteBringHere,
            onTap: () => unawaited(remoteBringHere(app, news.job, news.file)));
      case BridgeExportNewsKind.remoteFailed:
        app.postNotice('${_failed(news)} · ${remoteWhy(news.error)}',
            error: true);
      case BridgeExportNewsKind.arrived:
        app.postNotice(l10n.remoteNewsArrived(news.compName, news.otherName));
      // The status line already says how this computer's own ended.
      case BridgeExportNewsKind.exported || BridgeExportNewsKind.failed:
        break;
    }
  }

  /// What stopped short, as the computer it is told on sees it: a file Bring
  /// here couldn't fetch, which stays there done, an export done here for
  /// another computer, or one sent to another.
  String _failed(BridgeExportNews news) {
    final job = remoteJobs().where((j) => j.job == news.job).firstOrNull;
    if (news.error == 'not_found' ||
        (job != null && job.sent && job.stage == BridgeRemoteStage.done)) {
      return l10n.remoteFetchFailed(news.file);
    }
    if (job != null && !job.sent) {
      return l10n.remoteNewsFailedFor(news.compName, news.otherName);
    }
    return l10n.remoteNewsFailed(news.otherName, news.compName);
  }

  @override
  void dispose() {
    unawaited(_outbox?.cancel());
    unawaited(_news?.cancel());
    if (_started) {
      friendInvites.removeListener(_keysChanged);
      account.removeListener(_accountChanged);
      phones
        ..sayAlso = null
        ..itemAlso = null
        ..queueAlso = null
        ..heardAlso = null
        ..queueOn = null;
    }
    super.dispose();
  }
}
