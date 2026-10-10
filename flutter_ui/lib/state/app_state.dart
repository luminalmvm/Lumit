// The open document and the shell's view of it: LumitState, its status-bar
// notice, and the two small helpers that name the window and read a project
// path off the command line. Lifted out of main.dart unchanged.

import 'dart:async';
import 'dart:io' show Directory, File;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/shell/comp_settings_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/import.dart' show BridgeImportReport;
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/share.dart' as bridge_share;
import 'package:lumit_flutter/src/rust/api/share.dart'
    hide joinSharedProject, shareDefaultPort, shareLinkIn, shareRelayPort;
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:lumit_flutter/state/share.dart';
import 'package:lumit_flutter/state/ui_state.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:provider/provider.dart';

/// The window's title: plain 'Lumit' until the project has a home on disk,
/// then 'Lumit - `<file name>`' without the extension — the same convention as
/// every editor's title bar.
String windowTitleFor(String? path) {
  if (path == null || path.isEmpty) return 'Lumit';
  var name = path.split(RegExp(r'[/\\]')).last;
  if (name.toLowerCase().endsWith('.lum')) {
    name = name.substring(0, name.length - 4);
  }
  return 'Lumit - $name';
}

/// The `.lum` file a double-click or `lumit myproject.lum` asked us to open:
/// the first argument that ends in `.lum` and exists on disk, or null. The
/// Windows runner forwards the command line as entrypoint arguments (the
/// installer's file association passes the document path this way); anything
/// else on the line — flags, stray tokens — is not a project and is ignored.
///
/// A Linux desktop hands the document over as a `file:` address, since the
/// same launcher line also takes an invite link.
String? projectPathFromArgs(List<String> args) {
  for (var a in args) {
    if (a.startsWith('file:')) {
      try {
        a = Uri.parse(a).toFilePath();
      } catch (_) {
        continue;
      }
    }
    if (a.toLowerCase().endsWith('.lum') && File(a).existsSync()) return a;
  }
  return null;
}

/// The invite link Lumit was started with, as a click on one in a browser
/// starts it: the first argument that holds an invite, or null.
String? inviteFromArgs(List<String> args) {
  for (final a in args) {
    // Only what could be one is put to the engine: a path is not.
    if (!a.startsWith('lumit:') && !a.startsWith('https:')) continue;
    try {
      final link = bridge_share.shareLinkIn(text: a);
      if (link != null) return link;
    } catch (_) {
      // No engine to ask, as in a widget test.
    }
  }
  return null;
}

class LumitState extends ChangeNotifier {
  ProjectReference? project;

  /// The invite link Lumit was started with, until the Shared project window
  /// has opened on it.
  String? launchInvite;

  StreamSubscription? currentDocumentStream;

  /// The render worker's reply stream. Cancelled when another project is
  /// adopted, so a stale worker cannot feed frames to the new project's Viewer.
  StreamSubscription? workerStream;

  final StreamController<ScopedChange> _onChange = StreamController.broadcast();

  final StreamController<WorkerResponse> _onWorkerResponse =
      StreamController.broadcast();

  Stream<ScopedChange> get onChange => _onChange.stream;

  Stream<WorkerResponse> get onWorkerResponse => _onWorkerResponse.stream;

  /// The status bar's one-line notice: the latest quiet message or genuine
  /// error, dismissed by its close button. One current notice rather than a
  /// feed, which is what the egui shell's `app.notice` was too.
  final ValueNotifier<LumitNotice?> notice = ValueNotifier(null);

  /// The last twenty notices, newest first: what a click on the status bar's
  /// notice lists. Held while Lumit is open and never written anywhere.
  final List<LumitNotice> recentNotices = [];

  void postNotice(String message, {bool error = false}) {
    final posted = LumitNotice(message, error: error);
    recentNotices.insert(0, posted);
    if (recentNotices.length > 20) recentNotices.removeLast();
    notice.value = posted;
  }

  /// How the shell asks what to do with unsaved changes. It answers true when
  /// the project may go: it was saved, or the user chose to discard it. Null
  /// until there is a window to ask in.
  Future<bool> Function()? askUnsaved;

  /// How the shell offers back the edits a crash left behind, for the project
  /// at a path. Null until there is a window to ask in.
  Future<void> Function(String path)? offerRecovery;

  bool _askingUnsaved = false;

  /// Whether the open project has unsaved changes somebody can be asked about.
  /// Synchronous, so a clean project is swapped without waiting a turn.
  bool get unsavedNeedsAsking {
    if (askUnsaved == null) return false;
    try {
      return project?.isDirty() ?? false;
    } catch (_) {
      // A reference that has gone dead holds nothing left to save.
      return false;
    }
  }

  /// Ask about the unsaved changes, and say whether the project may go.
  ///
  /// Every way out of a project comes through here: New, Open, Close, an
  /// import, and quitting. A second request while the question is up is
  /// refused, so the question is never stacked on itself.
  Future<bool> askBeforeLeaving() async {
    final ask = askUnsaved;
    if (ask == null) return true;
    if (_askingUnsaved) return false;
    _askingUnsaved = true;
    try {
      return await ask();
    } finally {
      _askingUnsaved = false;
    }
  }

  /// A new, empty project in place of the open one. Close project is this too.
  ///
  /// [ask] false skips the question about unsaved changes, for a project that
  /// holds nothing of the user's.
  Future<void> newProject({bool ask = true}) async {
    if (ask && unsavedNeedsAsking && !await askBeforeLeaving()) return;
    _adopt(LumitBridgeState.newProject(onChangeStream: _changeSink()));
  }

  /// The shared project this is one end of, when it is.
  final ShareState share = ShareState();

  StreamSubscription<BridgeShareEvent>? _shareEvents;

  /// Share the open project from this machine, under [name], listening on
  /// [port]. [key] is the secret of the invite this project was last shared
  /// by, to make the same invite again, or null for a new one. [outside]
  /// asks the router to open the port for people outside this network, and
  /// [relay] is a relay to keep a room at for whoever that does not let in.
  /// [password] is one every guest has to give as well as holding the link,
  /// and null keeps whatever [key] was shared with.
  /// Null when there is no project open or it is already shared.
  BridgeShareStarted? startSharing(
      {required String name,
      required int port,
      String? key,
      String? password,
      bool outside = false,
      String? relay}) {
    final open = project;
    if (open == null || share.active) return null;
    final events = RustStreamSink<BridgeShareEvent>();
    final BridgeShareStarted started;
    try {
      started = open.share(
          name: name,
          port: port,
          key: key,
          password: password,
          outside: outside,
          relay: relay,
          events: events);
    } catch (_) {
      return null;
    }
    if (started case BridgeShareStarted_Sharing(:final port, :final restored)) {
      _shareEvents = events.stream.listen(_onShareEvent);
      share.begin(ShareRole.host, open, onPort: port);
      // The project was closed without saving, and the engine has put back
      // what everyone did after the last save.
      if (restored > 0) postNotice(l10n.shareRestored(restored));
    }
    return started;
  }

  /// Join the shared project [invite] names and make it the open one. It
  /// arrives unsaved, so saving writes this person's own copy. [password]
  /// is the host's, when it set one. [footage] is the folder this machine
  /// keeps the project's footage in.
  ///
  /// Null when another project is already on its way in. Anything else the
  /// caller shows: the previous project stays loaded unless this joined.
  Future<BridgeJoinOutcome?> joinShared(
      {required String invite,
      required String name,
      String? password,
      String? footage}) async {
    // One at a time, for [openProject]'s reason.
    if (opening.value) return null;
    opening.value = true;
    _openProgressWatch?.cancel();
    _openProgressWatch = null;
    openProgress.value = null;
    final events = RustStreamSink<BridgeShareEvent>();
    BridgeJoinOutcome outcome;
    try {
      outcome = await bridge_share.joinSharedProject(
          invite: invite,
          name: name,
          password: password,
          footage: footage,
          onChangeStream: _changeSink(),
          events: events);
    } catch (_) {
      outcome = const BridgeJoinOutcome.failed();
    }
    if (outcome case BridgeJoinOutcome_Joined(:final project)) {
      // Deliberately still `opening`: the document is in, the picture is not.
      _adopt(project);
      _shareEvents = events.stream.listen(_onShareEvent);
      share.begin(ShareRole.guest, project);
    } else {
      opening.value = false;
    }
    return outcome;
  }

  /// Stop sharing, or leave if someone else hosts. The project stays open.
  void stopSharing() {
    try {
      project?.stopSharing();
    } catch (_) {
      // Already closed, which stops it too.
    }
    _forgetShare();
  }

  /// The engine has let go of sharing without anyone being told, as it does
  /// when the document is swapped for a recovered one.
  void sharingLetGo() => _forgetShare();

  void _forgetShare() {
    _shareEvents?.cancel();
    _shareEvents = null;
    share.end();
  }

  void _onShareEvent(BridgeShareEvent event) {
    switch (event) {
      case BridgeShareEvent_People(:final people):
        share.setPeople(people);
      case BridgeShareEvent_Away():
        share.setAway(true);
        postNotice(l10n.shareAway);
      case BridgeShareEvent_Back(:final held, :final refused):
        share.setAway(false, conflicts: held);
        // The engine put the host's document in place of this one, so every
        // panel reads again, the way it does when a project is swapped.
        final open = project;
        if (open != null) {
          handleChange(ScopedChange(project: open, items: true));
        }
        postNotice(refused > 0 ? l10n.shareBackRefused(refused) : l10n.shareBack,
            error: refused > 0);
      case BridgeShareEvent_Elsewhere():
        postNotice(l10n.shareElsewhere, error: true);
      case BridgeShareEvent_Reach(:final reach):
        share.setReach(reach);
      case BridgeShareEvent_Relayed(:final relayed):
        share.setRelayed(relayed);
      case BridgeShareEvent_Ended(:final reason):
        stopSharing();
        postNotice(shareEndingText(reason),
            error: reason is! BridgeShareEnding_Closed);
    }
  }

  /// True from the moment a document starts being read until the Viewer has
  /// something to show of it. The shell draws its progress bar over the
  /// previous project and swaps nothing until this goes back to false.
  ///
  /// **It covers the picture, not just the read.** Reading the file is the
  /// first half; the second is the new project's render worker starting and
  /// serving a frame, and a shell that filled its panels between the two read
  /// as an editor that had loaded and then sat there. Everything appears
  /// together instead — which is what an application loading looks like.
  /// [previewReady] is what ends it.
  final ValueNotifier<bool> opening = ValueNotifier(false);

  /// How far the open has got, or null before the engine has said anything
  /// about it.
  ///
  /// The engine reports each phase of the read as it begins and hands over the
  /// share of the whole open that sits behind it; the last stretch — the render
  /// worker starting and answering — is the frontend's own, and [previewReady]
  /// closes it at 1. Weighted phases, not a timer: the card would rather move
  /// in four honest steps than sweep a bar that knows nothing.
  final ValueNotifier<OpenProgress?> openProgress = ValueNotifier(null);

  /// The open in progress reporting its phases, held so the next one can let
  /// it go.
  StreamSubscription<OpenProgress>? _openProgressWatch;

  /// Take a phase report, but never let the bar go backwards — a late report
  /// arriving behind a later one would otherwise pull the fill back.
  void _reportOpenProgress(OpenProgress progress) {
    final reached = openProgress.value?.fraction ?? -1;
    if (progress.fraction >= reached) openProgress.value = progress;
  }

  /// The line on the card shown over the shell while some other seconds-long
  /// job runs, or null when none is. Beat detection is the first of them.
  ///
  /// Separate from [opening] because the two say different things: [opening] is
  /// a document being swapped underneath the panels, this is the document
  /// standing still while something works on it. Set it through `showBusyWhile`
  /// (shell/splash.dart) so the card cannot be left up by a job that failed.
  final ValueNotifier<String?> busy = ValueNotifier(null);

  /// How far that job has got, 0..1, or null for one that cannot say.
  ///
  /// Beat detection can: the engine reports the share of the run behind it as
  /// it mixes the sources down, and closes at one once the markers are in. The
  /// card keeps whichever bar it opened with, so this is set before the card
  /// goes up and let go after it comes down.
  final ValueNotifier<double?> busyProgress = ValueNotifier(null);

  /// How that job is stopped, or null for one that cannot be. Set before the
  /// card goes up, like [busyProgress], and let go after it comes down.
  final ValueNotifier<VoidCallback?> busyCancel = ValueNotifier(null);

  /// The Viewer has something to show, or there is nothing for it to show —
  /// either way the shell can come out from behind its progress bar.
  ///
  /// Called on the first sign of life from the new project's worker (any reply
  /// at all, not only a frame: a project whose first render faults must not
  /// leave the interface covered), and by the session restore when it fronts no
  /// composition, which is a project with no picture to wait for.
  void previewReady() {
    if (!opening.value) return;
    // Full, then gone: the bar reaching its end is what "open" means, and a
    // card that vanished at eighty per cent would read as one that gave up.
    final last = openProgress.value;
    if (last != null) {
      openProgress.value = OpenProgress(phase: last.phase, fraction: 1);
    }
    opening.value = false;
  }

  /// [recover] false opens the file as it is, for the recovery dialogue's own
  /// opens, which must not ask the question again.
  Future<void> openProject(String path, {bool recover = true}) async {
    // One at a time: the change sink below is a single pending field, and two
    // opens in flight would have the second take the first's.
    if (opening.value) return;
    if (unsavedNeedsAsking && !await askBeforeLeaving()) return;
    // Asked again, because another open can have started while the question
    // was up.
    if (opening.value) return;
    opening.value = true;
    // Determinate from the first frame, before the engine has had a turn to
    // say so: the card must not flip from a sweeping bar to a filling one a
    // millisecond in. Nothing is claimed here — the fill is zero, and the
    // engine's own first report replaces this as it starts reading.
    openProgress.value =
        OpenProgress(phase: OpenPhase.readingFile, fraction: 0);
    // The engine's running commentary on the read. **Not cancelled when
    // the call returns**: a phase report crosses to Dart on an event-loop turn
    // of its own, so a subscription dropped the moment `openProject` resolved
    // would take every report still in flight with it and leave the bar stuck
    // at nought. It is let go at the start of the next open instead, by which
    // time the engine has long finished talking about this one.
    final progress = RustStreamSink<OpenProgress>();
    // The call is *started* before the sink is listened to, and that order is
    // not a style choice: a `RustStreamSink` has no stream until it has been
    // handed to a call, and asking for one early throws. Handing it over is
    // what opens the port; nothing is lost in between, because the stream
    // buffers what arrives before the first listener (`listenAndBuffer`). The
    // change sink beside it is attached the same way, after its own call.
    final shareEvents = RustStreamSink<BridgeShareEvent>();
    final pending = LumitBridgeState.openProject(
        path: path,
        onChangeStream: _changeSink(),
        onProgressStream: progress,
        shareEvents: shareEvents);
    _openProgressWatch?.cancel();
    _openProgressWatch = progress.stream.listen(_reportOpenProgress);
    // Null means the file would not open; the previous project stays loaded
    // rather than the app being left with none.
    final opened = await pending;
    if (opened == null) {
      postNotice(l10n.couldNotOpen(path), error: true);
      openProgress.value = null;
      opening.value = false;
      return;
    }
    // Deliberately still `opening`: the document is in, the picture is not.
    _adopt(opened);
    // A guest's own copy, closed while its host was away, opens still a guest
    // and still looking. What it has found since comes down the events.
    var guest = false;
    try {
      guest = opened.shareGuest();
    } catch (_) {
      // Closed again already.
    }
    if (guest) {
      _shareEvents = shareEvents.stream.listen(_onShareEvent);
      share.begin(ShareRole.guest, opened);
    } else {
      // Nothing will come down it, so its port is let go of.
      shareEvents.stream.listen((_) {}).cancel();
    }
    // A run that never closed this project left edits in its journal, which
    // is a crash. They are offered back before anything else is done to it.
    // A guest's copy has its own from the edits kept for it.
    if (recover && !guest) {
      var crashed = false;
      try {
        crashed = opened.endedBadly();
      } catch (_) {
        // Closed again already.
      }
      if (crashed) unawaited(offerRecovery?.call(path));
    }
  }

  /// Import an After Effects project and make it the open one (docs/11) —
  /// either front door: the `.aep` itself, or a Lumit Bridge bundle.
  ///
  /// Answers the report to show, or null when what was picked is not something
  /// this build can read — the previous project stays loaded in that case,
  /// exactly as it does for a `.lum` that will not open. **A report is not a
  /// failure**: an import always completes, and everything that could not be
  /// carried across is a row in it (docs/11 §9).
  Future<BridgeImportReport?> importAeBundle(String path) async {
    // One at a time, for [openProject]'s reason: `_pendingSink` is a single
    // field and two adoptions in flight would have the second take the first's.
    if (opening.value) return null;
    if (unsavedNeedsAsking && !await askBeforeLeaving()) return null;
    if (opening.value) return null;
    // Forgiveness before the engine sees the path: people naturally pick the
    // folder *containing* the bundle, not the bundle itself. One unambiguous
    // `.lum-bundle` child is what they meant. Presentation routing only — the
    // engine still decides whether what it is handed opens.
    var target = path;
    try {
      final dir = Directory(path);
      if (!File('$path/manifest.json').existsSync() && dir.existsSync()) {
        final bundles = dir
            .listSync()
            .whereType<Directory>()
            .where((d) => d.path.toLowerCase().endsWith('.lum-bundle'))
            .toList();
        if (bundles.length == 1) target = bundles.first.path;
      }
    } catch (_) {
      // Unreadable folder: the engine's own refusal will say so.
    }
    opening.value = true;
    // An import reports no phases, so its card sweeps rather than filling —
    // and must not inherit the fill, or the live reports, of the last open.
    _openProgressWatch?.cancel();
    _openProgressWatch = null;
    openProgress.value = null;
    final imported = await LumitBridgeState.importAeBundle(
        path: target, onChangeStream: _changeSink());
    if (imported == null) {
      // Three misses, three answers. An `.aep` the parser could not read is
      // the one the direct route made possible and the one worth being calm
      // about: a newer After Effects may store something this build has not
      // met, and the Bridge route reads it in full. A *folder* holding an
      // `.aep` is the older mistake — the bundle picker asked for a folder
      // and the user reasonably pointed it at the project's own — and still
      // teaches the route. Anything else is simply not a bundle.
      final aep = path.toLowerCase().endsWith('.aep');
      var folderOfAep = false;
      try {
        folderOfAep = !aep &&
            Directory(path)
                .listSync()
                .any((e) => e.path.toLowerCase().endsWith('.aep'));
      } catch (_) {}
      postNotice(
          aep
              ? l10n.aeAepUnreadable
              : folderOfAep
                  ? l10n.aeBundleFromAep
                  : l10n.aeCouldNotImport(path),
          error: true);
      opening.value = false;
      return null;
    }
    // Deliberately still `opening`: the document is in, the picture is not.
    _adopt(imported.project);
    return imported.report;
  }

  /// The sink Rust pushes scoped document changes down. Held for the call so
  /// [_adopt] can attach to the same one.
  RustStreamSink<ScopedChange>? _pendingSink;

  RustStreamSink<ScopedChange> _changeSink() =>
      _pendingSink = RustStreamSink<ScopedChange>();

  /// Take over a freshly created or opened project: start its render worker and
  /// subscribe to both of its streams.
  ///
  /// Both subscriptions matter and `newProject` used to make neither properly —
  /// it started the worker but dropped the returned stream, so no rendered frame
  /// ever reached the Viewer for a new project.
  void _adopt(ProjectReference opened) {
    // The project being replaced is closed, not abandoned: left in the
    // engine's registry it would keep its render worker — and that worker's
    // whole GPU device — alive for as long as the process runs. `openProject`
    // already cleared the registry wholesale before this runs, and close is
    // idempotent, so the open path pays nothing for the repeat.
    final previous = project;
    if (previous != null && previous.internalid != opened.internalid) {
      previous.close();
    }
    // The engine stops sharing a project that is replaced or closed.
    _forgetShare();
    project = opened;
    // The comp list is cached per document and invalidated when the
    // item tree changes — but adopting another project is not a change to the
    // tree, it is a different tree. Left standing, every reader of `comps()`
    // answers from the project that is no longer loaded until something
    // happens to edit the new one: the session restore looked the reopened
    // project's comps up in the *previous* project's list and found none of
    // them, so a reopened project came back with no tabs and nothing fronted.
    _compsCache = null;

    workerStream?.cancel();
    workerStream =
        opened.startWorker().listen((msg) => _onWorkerResponse.add(msg));

    final sink = _pendingSink;
    if (sink != null) {
      currentDocumentStream?.cancel();
      currentDocumentStream = sink.stream.listen(handleChange);
      _pendingSink = null;
    }

    refreshWindowTitle();
    // **The swap is published, not just performed**. `_compsCache`
    // above is one of many per-document caches, and it was the only one told:
    // the Project panel's item and name caches, the comp read model and the
    // comp-time cache all drop themselves on an `items` change and nothing
    // else, so a swap that says nothing leaves every one of them answering
    // from the project this method has just closed. Reading a closed project's
    // handle throws, and a build that throws is a blank panel.
    //
    // Last, after the worker is up: a subscriber rebuilds on this, and it
    // should rebuild against a project whose worker is answering. `handleChange`
    // notifies as well, which is what this replaces.
    handleChange(ScopedChange(project: opened, items: true));
    unawaited(_backfillThumbnail());
  }

  /// A project opened with no picture on file grows one, once.
  ///
  /// Every save has filed a thumbnail since the engine could draw one, but
  /// projects that predate it — an After Effects conversion, everything saved
  /// before — have none, and their welcome rows are empty wells until the next
  /// save. This is the one place that fixes them: the first time such a project
  /// is opened, its first composition is drawn and filed.
  ///
  /// Deliberately at the end of adopting, and deliberately not awaited: it is
  /// nobody's business but the welcome screen's, and an open must not wait for
  /// a picture of the project it has already loaded. A project with a picture
  /// already, or with no path to key one by, costs one `existsSync`.
  Future<void> _backfillThumbnail() async {
    try {
      final path = project?.path();
      if (path == null || Workspace.thumbnailFile(path).existsSync()) return;
      final comps = this.comps();
      if (comps.isEmpty) return;
      // Frame 0 of the first comp: nothing has been fronted yet at this point
      // in an open, and the session restore that will front one runs after.
      await Workspace.fileCompThumbnail(path, comps.first.$1);
    } catch (_) {
      // A project whose picture would not draw keeps its placeholder, which is
      // exactly the state it was already in.
    }
  }

  /// Put the project's name in the title bar. Called when the document's path
  /// can have changed — adopting a project, and a completed save — rather than
  /// on every edit, so no bridge call rides the change stream.
  void refreshWindowTitle() {
    SystemChrome.setApplicationSwitcherDescription(
      ApplicationSwitcherDescription(label: windowTitleFor(project?.path())),
    );
  }

  /// Tell the app an edit landed, for callers that made one themselves rather
  /// than learning about it from the engine's change stream.
  ///
  /// The stream is the right mechanism for edits made *elsewhere*, but a caller
  /// that just performed an op should not wait for a Rust→Dart round trip to see
  /// its own result — see the same reasoning in project_panel_frb.dart.
  void notifyDocumentChanged() => notifyListeners();

  /// Give [layer] a Retime, or take it away again — the one implementation,
  /// shared by the keyboard chords and the Composition menu (docs/04
  /// §12), so no route can drift from the others.
  ///
  /// The engine refuses nothing here, but the call is a bridge crossing like
  /// any other: a layer deleted between the menu opening and the click would
  /// throw, and a command that cannot be performed should do nothing rather
  /// than take the interface down with it.
  bool toggleRetime(LayerReference layer) {
    try {
      layer.toggleRetimeProperty();
    } catch (_) {
      return false;
    }
    notifyDocumentChanged();
    return true;
  }

  /// Import footage into the open project, and say whether anything landed.
  ///
  /// Here rather than in the menu bar because the Project panel offers the same
  /// command, and two copies of "import each path, then notify" is one copy too
  /// many for something every new user's first action goes through.
  /// A batch is **one** undo step: picking six files in the dialogue,
  /// or dropping six on the panel, is one action the user took, so it is one
  /// Ctrl-Z. The group is closed in a `finally` because a group left open
  /// records nothing.
  Future<bool> importFootagePaths(List<String> paths) async {
    final project = this.project;
    if (project == null || paths.isEmpty) return false;
    final group = paths.length > 1;
    if (group) project.beginUndoGroup();
    try {
      for (final path in paths) {
        // A layered document comes in as a composition of its layers. The
        // engine says which files those are, and everything else is footage.
        final leftOut = project.importLayers(path: path);
        if (leftOut == null) {
          project.importFootage(path: path);
        } else if (leftOut > 0) {
          postNotice(l10n.importLayersLeftOut(leftOut));
        }
      }
    } finally {
      if (group) project.endUndoGroup();
    }
    notifyDocumentChanged();
    return true;
  }

  /// Make a composition, asking for its settings first.
  ///
  /// Every route to a new comp — the menu bar, the command palette, the Project
  /// panel's button, and footage dropped on that button — comes through here, so
  /// there is one answer to what "New composition" does. `footage` is what was
  /// dropped: the dialog opens on the media's own size, rate and length, and each
  /// item lands in the finished comp as a layer.
  ///
  /// Null when the project is closed or the dialog was cancelled.
  Future<CompositionReference?> newComposition(
    BuildContext context, {
    List<FootageReference> footage = const [],
  }) async {
    final project = this.project;
    if (project == null) return null;
    final comp = await showNewCompositionFrb(
      context: context,
      project: project,
      footage: footage,
      asSequence: Provider.of<LumitUiState>(context, listen: false)
          .workspace
          .interface
          .videoAsSequenceLayer,
    );
    if (comp == null) return null;
    notifyDocumentChanged();
    return comp;
  }

  /// Make a node graph composition and front it, asking for its settings first
  /// (docs/impl/node-graph-comp.md §4.4).
  ///
  /// The Composition menu, the command palette and the Project panel's context
  /// menu all come through here, so there is one answer to what "New node
  /// graph" does. It fronts what it made, because a graph is worked on in the
  /// Node graph panel and the panel draws the comp in front.
  ///
  /// Null when the project is closed or the dialogue was cancelled.
  Future<CompositionReference?> newNodeGraph(BuildContext context) async {
    final project = this.project;
    if (project == null) return null;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final comp = await showNewCompositionFrb(
      context: context,
      project: project,
      nodeGraph: true,
    );
    if (comp == null) return null;
    ui.setSelectedComp(comp);
    notifyDocumentChanged();
    return comp;
  }

  void handleChange(ScopedChange event) {
    // The item tree changed shape: the cached comp list is stale.
    //
    // **The item flag alone, not `item != null`**. This cache holds
    // every comp and its *name*, and only the item scope can change either:
    // `op_scope` sets it for adding, removing and renaming an item, and for
    // comp settings, which carry the name. A layer op names its comp too — and
    // that dropped the cache on every switch, every keyframe, every nudge, so
    // the next build of the comp-tab strip re-read `get_settings` for all
    // forty-eight comps in the project (docs/impl/ui-performance.md §4.5).
    if (event.items) _compsCache = null;

    _onChange.add(event);

    // A change that names a subtree is that subtree's business: the comp read
    // model and ProjectItemBuilder subscribe to the stream themselves.
    if (event.layer != null || event.item != null) return;

    _compsCache = null;
    // Nothing narrower to aim at — whoever listens to LumitState rebuilds.
    notifyListeners();
  }

  /// Every composition in the project with its name, folders walked — cached
  /// so the comp tabs cost no bridge calls per rebuild. Invalidated
  /// whenever the item tree changes.
  List<(CompositionReference, String)>? _compsCache;
  List<(CompositionReference, String)> comps() {
    if (_compsCache != null) return _compsCache!;
    final out = <(CompositionReference, String)>[];
    void walk(List<ItemReference> items) {
      for (final item in items) {
        switch (item) {
          case ItemReference_Composition(:final field0):
            out.add((field0, field0.getSettings().name));
          case ItemReference_Folder(:final field0):
            walk(field0.getChildren());
          case _:
            break;
        }
      }
    }

    walk(project?.getItems() ?? const []);
    return _compsCache = out;
  }
}

/// One status-bar notice: what to say, and whether it is a genuine error
/// (drawn in the warning tint) rather than quiet feedback.
class LumitNotice {
  final String message;
  final bool error;
  const LumitNotice(this.message, {this.error = false});
}
