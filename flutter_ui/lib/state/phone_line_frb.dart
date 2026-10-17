// The line between this computer and the account's phones.
//
// While the account has Pro, Lumit holds one socket to the account's room on
// the server and says there, once a second, what it is exporting and what it
// could, so a phone can show it. A phone joins the account by scanning a code
// this computer shows ([pair]), and the room says when one has. A phone may
// start, cancel and remove exports through the room too, and have a finished
// one sent as a preview, unless Settings says not to ([_command]).
//
// Only names go up: the project's and each export's file, never a path, and
// nothing a phone sends is used as one. Nothing here is on a rebuild path.

import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart'
    show workAreaFrames;
import 'package:lumit_flutter/shell/about_window_frb.dart' show lumitVersion;
import 'package:lumit_flutter/shell/export_dialog_frb.dart'
    show exportDefaultPath;
import 'package:lumit_flutter/shell/status_line_frb.dart'
    show statusLineExportStarted;
import 'package:lumit_flutter/src/rust/api/export.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/state.dart' show ScopedChange;

import 'account.dart';
import 'app_state.dart';
import 'reviews.dart';
import 'updates.dart' show versionFromBootLine;
import 'workspace.dart';

/// A computer or phone signed in to the account. [me] is this computer.
typedef AccountDevice = ({String id, String name, bool phone, bool me});

class PhoneLine extends ChangeNotifier {
  final AccountState account;
  final LumitState app;
  final Workspace workspace;

  /// What sends a preview, for an export a phone asks to have sent as one.
  final ReviewsState reviews;

  PhoneLine(this.account, this.app, this.workspace, this.reviews);

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

  /// The open project's compositions as said, read again after the project
  /// or anything in it changes rather than every second.
  List<Map<String, Object>>? _comps;
  ProjectReference? _compsOf;
  StreamSubscription<ScopedChange>? _changes;

  /// When each export was first seen running, and first seen finished, in
  /// unix seconds, by its id in the queue.
  final Map<int, int> _ran = {}, _ended = {};

  /// Begin following the account. Not called by a test, which keeps a test
  /// run off the network.
  void start() {
    if (_started) return;
    _started = true;
    _changes = app.onChange.listen((_) => _comps = null);
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
      'compositions': _compositions(),
      // A small file, read again each time so a preset saved shows at once.
      'presets': [for (final preset in exportPresetList()) preset.name],
      'remote': workspace.phonesExport,
    });
    if (said == _said) return;
    _said = said;
    socket.add(said);
  }

  /// Each by the id it is found again by, with whether it has a work area.
  List<Map<String, Object>> _compositions() {
    final project = app.project;
    if (_comps != null && project == _compsOf) return _comps!;
    _compsOf = project;
    try {
      return _comps = [
        for (final (comp, name) in app.comps())
          {
            'id': '${comp.internalid}',
            'name': name,
            'workArea': comp.getWorkArea() != null,
          },
      ];
    } catch (_) {
      // Closed meanwhile.
      return _comps = [];
    }
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
    if (j is Map && j['type'] == 'command') _command(j);
  }

  /// Do what a phone asked, and always answer it: ok, or why not. Said on the
  /// status line like anything else done here, and nothing opens.
  void _command(Map<dynamic, dynamic> j) {
    String? error;
    try {
      error = workspace.phonesExport ? _do(j) : 'turned_off';
    } catch (_) {
      error = 'failed';
    }
    _live?.add(jsonEncode({
      'type': 'reply',
      'id': j['id'],
      'to': j['from'],
      if (error == null) 'ok': true else 'error': error,
    }));
    // The queue as it is now, rather than at the next beat.
    _say();
  }

  String? _do(Map<dynamic, dynamic> j) {
    final what = j['do'];
    if (what == 'start') {
      exportQueueStart();
      statusLineExportStarted.value++;
      app.postNotice(l10n.phoneStartedQueue);
      return null;
    }
    if (what == 'queue') return _queue(j);
    if (what != 'cancel' && what != 'remove' && what != 'preview') {
      return 'failed';
    }
    final item = exportQueueList().where((i) => i.id == j['item']).firstOrNull;
    if (item == null) return 'not_found';
    if (what == 'preview') return _preview(item);
    if (what == 'cancel') {
      exportQueueCancel(id: item.id);
      app.postNotice(l10n.phoneCancelledExport(item.compName));
    } else {
      exportQueueRemove(id: item.id);
      app.postNotice(l10n.phoneRemovedExport(item.compName));
    }
    return null;
  }

  /// Add a composition to the queue with a saved preset and start it, as the
  /// export dialogue's Export button would with that preset chosen: to where
  /// Settings > Export says, named as it names it, never over a file there.
  String? _queue(Map<dynamic, dynamic> j) {
    final project = app.project;
    if (project == null) return 'no_project';
    final comp = app
        .comps()
        .where((c) => '${c.$1.internalid}' == j['composition'])
        .firstOrNull
        ?.$1;
    final preset = j['preset'];
    if (comp == null || preset is! String || preset.isEmpty) return 'not_found';
    final stored = exportPresetGet(name: preset);
    if (stored == null) return 'not_found';
    final settings = comp.getSettings();
    var path = exportDefaultPath(exportDefaultsGet(),
        preset: preset,
        compName: settings.name,
        codec: stored.codec,
        projectPath: project.path());
    if (path == null) return 'no_folder';
    if (exportPathTaken(path: path, codec: stored.codec)) {
      path = exportFreePath(path: path, codec: stored.codec);
    }
    final area = j['range'] == 'whole' ? null : workAreaFrames(comp);
    final start = area?.start ?? 0;
    final end = area?.end ?? comp.durationFrames();
    // The dialogue's fields as the preset fills them. Its own ticks are off,
    // the Viewer's region is not in it, and it sends the size the frame
    // comes out at unless that is the composition's own.
    BridgeExportSpec spec(int width, int height) => BridgeExportSpec(
          preset: preset,
          codec: stored.codec,
          width: width,
          height: height,
          bitrateMbps: stored.bitrateMbps,
          peakMbps: stored.peakMbps,
          bitrateAuto: stored.bitrateAuto,
          fps: stored.fps,
          rangeStartFrame: start,
          rangeEndFrame: end,
          includeAudio: stored.includeAudio,
          audioBitRate: stored.audioBitRate,
          audioRate: stored.audioRate,
          audioDepth: stored.audioDepth,
          audioChannels: stored.audioChannels,
          depth: stored.depth,
          alphaChannel: stored.alphaChannel,
          straightAlpha: stored.straightAlpha,
          colourSpace: stored.colourSpace,
          resample: stored.resample,
          cropTop: stored.cropTop,
          cropLeft: stored.cropLeft,
          cropBottom: stored.cropBottom,
          cropRight: stored.cropRight,
          useRegionOfInterest: false,
          region: Float64List(0),
          metadata: const [],
          qualityDivisor: stored.qualityDivisor,
          diskCacheReadOnly: stored.diskCacheReadOnly,
          effects: stored.effects,
          honourSolo: stored.honourSolo,
          renderGuides: stored.renderGuides,
          motionBlur: stored.motionBlur,
          retimeBlend: stored.retimeBlend,
          useProxies: stored.useProxies,
          makeANoise: false,
          openFolder: false,
        );
    var (width, height) = (stored.width, stored.height);
    if (width <= 0 || height <= 0) {
      final crop = exportCropFor(
          spec: spec(0, 0),
          compWidth: settings.width,
          compHeight: settings.height);
      final divisor = stored.qualityDivisor.clamp(1, 4);
      width = (crop.width / divisor).round().clamp(1, 16384);
      height = (crop.height / divisor).round().clamp(1, 16384);
    }
    final own = width == settings.width && height == settings.height;
    try {
      comp.queueExport(
          spec: spec(own ? 0 : width, own ? 0 : height),
          path: path,
          start: true);
    } catch (_) {
      return 'failed';
    }
    statusLineExportStarted.value++;
    app.postNotice(l10n.phoneQueuedExport(settings.name));
    // A preview is its own export, made and sent the way Send as a preview
    // does it, behind this one in the queue.
    if (j['preview'] == true) {
      unawaited(reviews.send(comp,
          into: reviews.linkOf(comp),
          title: settings.name,
          edge: 1080,
          start: start,
          end: end,
          project: project.path() ?? ''));
    }
    return null;
  }

  /// Send a finished export as a preview, the way [_queue] does: exported
  /// again at preview size, since the file itself may be any format and size.
  /// The queue keeps the composition's name and range rather than its id, so
  /// it is found by name, and a range left to the default is the work area.
  String? _preview(BridgeExportQueueItem item) {
    if (item.state is! BridgeExportQueueState_Done) return 'failed';
    final project = app.project;
    // ponytail: two compositions of the same name take the first. Carry the
    // id on the queue item if that bites.
    final comp = project == null
        ? null
        : app.comps().where((c) => c.$2 == item.compName).firstOrNull?.$1;
    if (comp == null) return 'no_project';
    final area = item.rangeStartFrame < 0 ? workAreaFrames(comp) : null;
    final start = area?.start ?? item.rangeStartFrame;
    final end = area?.end ?? item.rangeEndFrame;
    unawaited(reviews.send(comp,
        into: reviews.linkOf(comp),
        title: item.compName,
        edge: 1080,
        start: start,
        end: end,
        project: project!.path() ?? ''));
    app.postNotice(l10n.phoneSentPreview(item.compName));
    return null;
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
    unawaited(_changes?.cancel());
    unawaited(_live?.close());
    if (_started) account.removeListener(_accountChanged);
    super.dispose();
  }
}
