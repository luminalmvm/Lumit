// The line between this computer and the account's phones.
//
// While the account has Pro, Lumit holds one socket to the account's room on
// the server and says there, once a second, what it is exporting, so a phone
// can show it. A phone joins the account by scanning a code this computer
// shows ([pair]), and the room says when one has.
//
// Only names go up: the project's and each export's file, never a path.
// Nothing here is on a rebuild path.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/shell/about_window_frb.dart' show lumitVersion;
import 'package:lumit_flutter/src/rust/api/export.dart';

import 'account.dart';
import 'app_state.dart';
import 'updates.dart' show versionFromBootLine;

/// A computer or phone signed in to the account. [me] is this computer.
typedef AccountDevice = ({String id, String name, bool phone, bool me});

class PhoneLine extends ChangeNotifier {
  final AccountState account;
  final LumitState app;

  PhoneLine(this.account, this.app);

  /// The phone that last took the code this computer showed, by name. The
  /// Add a phone window clears it as it opens.
  String? justPaired;

  Timer? _beat, _retry;
  WebSocket? _live;
  bool _started = false, _gone = false;

  /// The account the line is held for, or null while there should be none.
  String? _for;
  int _look = 0;

  /// The last thing said, so the same thing is not said again.
  String? _said;

  /// When each export was first seen running, and first seen finished, in
  /// unix seconds, by its id in the queue.
  final Map<int, int> _ran = {}, _ended = {};

  /// Begin following the account. Not called by a test, which keeps a test
  /// run off the network.
  void start() {
    if (_started) return;
    _started = true;
    account.addListener(_accountChanged);
    _accountChanged();
  }

  /// Another account, none, or Pro come or gone: the line there was is not
  /// this one's.
  void _accountChanged() {
    final who = account.pro ? account.account?.id : null;
    if (who == _for) return;
    _for = who;
    _look++;
    _beat?.cancel();
    _retry?.cancel();
    unawaited(_live?.close());
    _live = null;
    if (who != null) unawaited(_listen(0, _look));
  }

  Uri get _room {
    final base = Uri.parse(cloudUrl);
    return base.replace(
        scheme: base.scheme == 'http' ? 'ws' : 'wss', path: '/v1/room');
  }

  /// One that drops is tried again, a little later each time, for as long as
  /// the account has Pro.
  Future<void> _listen(int tries, int look) async {
    var joined = false;
    try {
      final token = await account.accessToken();
      final socket = await WebSocket.connect('$_room', headers: {
        if (token != null) 'Authorization': 'Bearer $token',
      }).timeout(const Duration(seconds: 10));
      if (_gone || look != _look) {
        unawaited(socket.close());
        return;
      }
      _live = socket;
      joined = true;
      socket.pingInterval = const Duration(seconds: 20);
      _said = null;
      _say();
      // Once a second, which is as often as the server will hear one.
      _beat = Timer.periodic(const Duration(seconds: 1), (_) => _say());
      await for (final message in socket) {
        if (message is String) _heard(message);
      }
    } on WebSocketException {
      // Turned away: Pro may have ended, or the sign-in with it. Reading the
      // account again finds out, and closes the line if so.
      if (!joined) unawaited(account.refreshAccount());
    } catch (_) {
      // Offline, or the line dropped.
    }
    if (_gone || look != _look) return;
    _beat?.cancel();
    _live = null;
    _retry = Timer(Duration(seconds: math.min(30, 2 << tries)),
        () => _listen(joined ? 1 : tries + 1, look));
  }

  /// What this computer is and what it is exporting, when that has changed.
  void _say() {
    final socket = _live;
    if (socket == null) return;
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final items = exportQueueList();
    final ids = {for (final item in items) item.id};
    _ran.removeWhere((id, _) => !ids.contains(id));
    _ended.removeWhere((id, _) => !ids.contains(id));
    final said = jsonEncode({
      'type': 'state',
      'platform': Platform.operatingSystem,
      'version': versionFromBootLine(lumitVersion()) ?? '',
      'project': projectNameFor(app.project?.path()),
      'queue': [for (final item in items) _item(item, now)],
    });
    if (said == _said) return;
    _said = said;
    socket.add(said);
  }

  Map<String, Object?> _item(BridgeExportQueueItem item, int now) {
    final state = item.state;
    if (state is BridgeExportQueueState_Running) {
      _ran.putIfAbsent(item.id, () => now);
    }
    if (state is BridgeExportQueueState_Done ||
        state is BridgeExportQueueState_Failed) {
      _ended.putIfAbsent(item.id, () => now);
    }
    final running = state is BridgeExportQueueState_Running ? state : null;
    return {
      'id': item.id,
      'name': item.compName,
      'file': item.path.split(RegExp(r'[/\\]')).last,
      'preset': item.preset,
      'codec': item.codec,
      'state': switch (state) {
        BridgeExportQueueState_Waiting() => 'waiting',
        BridgeExportQueueState_Running() => 'running',
        BridgeExportQueueState_Done() => 'done',
        BridgeExportQueueState_Failed() => 'failed',
      },
      'frame': running?.frame.toInt() ?? 0,
      'total': running?.total.toInt() ?? 0,
      'error': state is BridgeExportQueueState_Failed ? state.error : null,
      'started': _ran[item.id],
      'ended': _ended[item.id],
    };
  }

  void _heard(String message) {
    final Object? j;
    try {
      j = jsonDecode(message);
    } catch (_) {
      return;
    }
    if (j is Map && j['type'] == 'paired' && j['name'] is String) {
      justPaired = j['name'] as String;
      notifyListeners();
    }
  }

  /// A code for a phone to scan, at its address, and when it runs out. Asking
  /// again replaces the one before.
  Future<({String url, DateTime expires})> pair() async {
    final j = await account.call('POST', '/v1/pair');
    final expires = j['expires'];
    return (
      url: '${j['url']}',
      expires: DateTime.fromMillisecondsSinceEpoch(
          (expires is int ? expires : 0) * 1000),
    );
  }

  Future<List<AccountDevice>> devices() async {
    final j = await account.call('GET', '/v1/devices');
    return [
      if (j['devices'] case final List<dynamic> all)
        for (final d in all)
          if (d is Map)
            (
              id: '${d['id']}',
              name: '${d['name']}',
              phone: d['phone'] == true,
              me: d['me'] == true,
            ),
    ];
  }

  Future<void> rename(String id, String name) =>
      account.call('PATCH', '/v1/devices/$id', body: {'name': name});

  Future<void> forget(String id) => account.call('DELETE', '/v1/phones/$id');

  @override
  void dispose() {
    _gone = true;
    _beat?.cancel();
    _retry?.cancel();
    unawaited(_live?.close());
    if (_started) account.removeListener(_accountChanged);
    super.dispose();
  }
}
