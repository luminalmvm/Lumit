// The open document and the shell's view of it: LumitState, its status-bar
// notice, and the two small helpers that name the window and read a project
// path off the command line. Lifted out of main.dart unchanged.

import 'dart:async';
import 'dart:io' show Directory, File, exit;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/shell/comp_settings_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/import.dart'
    show BridgeImportReport, cancelImport;
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/share.dart' as bridge_share;
import 'package:lumit_flutter/src/rust/api/share.dart'
    hide joinSharedProject, shareDefaultPort, shareLinkIn, shareRelayPort;
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:lumit_flutter/state/settings.dart';
import 'package:lumit_flutter/state/share.dart';
import 'package:lumit_flutter/state/ui_state.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:provider/provider.dart';

/// The window's title: plain 'Lumit' until the project has a home on disk,
/// then 'Lumit - `<file name>`' without the extension — the same convention as
/// every editor's title bar.
String windowTitleFor(String? path) {
  final name = projectNameFor(path);
  return name == null ? 'Lumit' : 'Lumit - $name';
}

/// The project's file name without the extension, or null until it has a home
/// on disk.
String? projectNameFor(String? path) {
  if (path == null || path.isEmpty) return null;
  final name = path.split(RegExp(r'[/\\]')).last;
  return name.toLowerCase().endsWith('.lum')
      ? name.substring(0, name.length - 4)
      : name;
}

/// The `.lum` file a double-click or `lumit myproject.lum` asked us to open:
/// the first argument that ends in `.lum` and exists on disk, or null. The
/// Windows runner forwards the command line as entrypoint arguments (the
/// installer's file association passes the document path this way); anything
/// else on the line — flags, stray tokens — is not a project and is ignored.
///
/// A Linux desktop hands the document over as a `file:` address, since the
/// same launcher line also takes an invite link.
String? projectPathFromArgs(List<String> args) => _fileFromArgs(args, '.lum');

/// The `.lfx` extension package a double-click asked Lumit to install, read
/// as [projectPathFromArgs] reads a project.
String? packagePathFromArgs(List<String> args) => _fileFromArgs(args, '.lfx');

String? _fileFromArgs(List<String> args, String ending) {
  for (var a in args) {
    if (a.startsWith('file:')) {
      try {
        a = Uri.parse(a).toFilePath();
      } catch (_) {
        continue;
      }
    }
    if (a.toLowerCase().endsWith(ending) && File(a).existsSync()) return a;
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

/// The macOS runner's end of this is `OpenRequests` in AppDelegate.swift.
/// The Windows and Linux runners answer it too, for the open Lumit a second
/// start hands its arguments to.
const _openRequests = MethodChannel('lumit/open');

/// What macOS has asked Lumit to open and nobody has taken yet: a project
/// double-clicked in Finder as its path, an invite link clicked in a browser
/// as it was written. macOS hands these over as events and never as
/// arguments, so they are read as arguments once they are here, by
/// [projectPathFromArgs] and [inviteFromArgs]. Elsewhere it is what a second
/// start passed on, and empty at start-up.
Future<List<String>> takeOpenRequests() async {
  try {
    return await _openRequests.invokeListMethod<String>('take') ?? const [];
  } catch (_) {
    // No runner to ask, as in a widget test.
    return const [];
  }
}

/// Hand [open] whatever Lumit is asked to open from here on. A click that
/// finds Lumit running comes to it on macOS. On the other systems a package
/// always does, and the rest when Lumit is kept running in the background.
void listenForOpenRequests(void Function(List<String> asked) open) {
  Future<void> take() async {
    final asked = await takeOpenRequests();
    if (asked.isNotEmpty) open(asked);
  }

  // The runner says only that there is something, and it is taken.
  _openRequests.setMethodCallHandler((_) => take());
  // Once now as well, for a request that came after start-up took its own.
  take();
}

/// Call [asked] when something is waiting to be opened, leaving it for
/// whoever calls [listenForOpenRequests] next.
void noticeOpenRequests(VoidCallback asked) {
  _openRequests.setMethodCallHandler((_) async => asked());
}

class LumitState extends ChangeNotifier {
  ProjectReference? project;

  /// Closes an undo step left open across time, a camera's run of wheel
  /// notches, so Undo, Redo and the History window never land inside it.
  VoidCallback? closeOpenStep;

  /// Close any undo step still open, before the history moves.
  void settleHistory() {
    final close = closeOpenStep;
    closeOpenStep = null;
    close?.call();
  }

  /// The invite link Lumit was started with, until the Shared project window
  /// has opened on it.
  String? launchInvite;

  /// The extension package Lumit was started with, until its install
  /// window has opened.
  String? launchPackage;

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

  /// The line each extension keeps on the bottom strip, by its id, with its
  /// name in front. Shown while there is no notice. Never saved.
  final ValueNotifier<Map<String, String>> extensionStatus =
      ValueNotifier(const {});

  void setExtensionStatus(String id, String? line) {
    if (extensionStatus.value[id] == line) return;
    final now = {...extensionStatus.value}..remove(id);
    if (line != null) now[id] = line;
    extensionStatus.value = now;
  }

  /// The last twenty notices, newest first: what a click on the status bar's
  /// notice lists. Held while Lumit is open and never written anywhere.
  final List<LumitNotice> recentNotices = [];

  void postNotice(String message,
      {bool error = false,
      bool sticky = false,
      String? action,
      VoidCallback? onTap}) {
    final posted = LumitNotice(message,
        error: error, sticky: sticky, action: action, onTap: onTap);
    // One that stays until it is dismissed keeps the line from anything
    // quiet, which waits behind it in the list a press on the line shows.
    final kept = notice.value;
    final behind = kept != null && kept.sticky && !error && !sticky;
    recentNotices.insert(behind && recentNotices.isNotEmpty ? 1 : 0, posted);
    if (recentNotices.length > 20) recentNotices.removeLast();
    if (!behind) notice.value = posted;
  }

  /// How the shell asks what to do with unsaved changes. It answers true when
  /// the project may go: it was saved, or the user chose to discard it. Null
  /// until there is a window to ask in.
  Future<bool> Function()? askUnsaved;

  /// How the shell offers back the edits a crash left behind, for the project
  /// at a path. Null until there is a window to ask in.
  Future<void> Function(String path)? offerRecovery;

  /// How the shell runs the script chosen for [ScriptEvent], told the file
  /// the event was about. Null until there is a window to run it in.
  void Function(ScriptEvent event, String path)? runEventScript;

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

  /// Where the user's project template is, or null when they have none. Set
  /// by the shell, which holds the setting. Null until there is one.
  String? Function()? projectTemplate;

  /// A new project in place of the open one: a copy of the user's template
  /// when they have set one, else an empty one. Close project is this too,
  /// with [blank], since closing is not asking for the template.
  ///
  /// [ask] false skips the question about unsaved changes, for a project that
  /// holds nothing of the user's.
  Future<void> newProject({bool ask = true, bool blank = false}) async {
    if (ask && unsavedNeedsAsking && !await askBeforeLeaving()) return;
    final template = blank ? null : projectTemplate?.call();
    if (template != null) {
      // Reading the template takes a moment, and the user may open something
      // else in it. What they opened stands, and the template is let go.
      final was = project;
      final sink = _changeSink();
      try {
        final outcome = await LumitBridgeState.newProjectFrom(
            path: template, onChangeStream: sink);
        if (outcome case BridgeOpened_Opened(project: final made)) {
          if (!identical(project, was) || opening.value) {
            if (identical(_pendingSink, sink)) _pendingSink = null;
            made.close();
            return;
          }
          return _adopt(made);
        }
      } catch (_) {
        // Falls through to an empty project, with the notice below.
      }
      if (!identical(project, was) || opening.value) return;
      postNotice(l10n.templateNotOpened, error: true);
    }
    _adopt(LumitBridgeState.newProject(onChangeStream: _changeSink()));
  }

  /// The shared project this is one end of, when it is.
  final ShareState share = ShareState();

  StreamSubscription<BridgeShareEvent>? _shareEvents;

  /// Until when losing the server and finding it again goes unsaid.
  DateTime _quietUntil = DateTime(0);

  /// The host of a project kept online is moving it to a new invite, which
  /// its own machine sees as losing the server and finding it again, once
  /// or twice over. That is the host's own doing, so for the next few
  /// seconds nothing is said of it.
  void quietReturn() =>
      _quietUntil = DateTime.now().add(const Duration(seconds: 10));

  bool get _quiet => DateTime.now().isBefore(_quietUntil);

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
      _shareEvents = events.stream.listen(handleShareEvent);
      share.begin(ShareRole.host, open, onPort: port);
      // The project was closed without saving, and the engine has put back
      // what everyone did after the last save.
      if (restored > 0) postNotice(l10n.shareRestored(restored));
    }
    return started;
  }

  /// Keep the open project online, under [name]: Lumit's server holds it,
  /// sealed, and puts everyone's edits in order, so anyone with the invite
  /// can open it when nobody else is there. [password] is one everyone has
  /// to give as well as holding the link. The door to the server has to be
  /// open first. Null when there is no project open or it is already shared.
  Future<BridgeOnlineStarted?> keepOnline(
      {required String name, String? password}) async {
    final open = project;
    if (open == null || share.active) return null;
    final events = RustStreamSink<BridgeShareEvent>();
    BridgeOnlineStarted? started;
    try {
      started = await open.shareOnline(
          name: name, password: password, events: events);
    } catch (_) {
      // Closed while it was being stored.
    }
    if (started is BridgeOnlineStarted_Online && identical(project, open)) {
      _shareEvents = events.stream.listen(handleShareEvent);
      share.begin(ShareRole.host, open);
    } else {
      // Nothing will come down it, so its port is let go of.
      events.stream.listen((_) {}).cancel();
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
    // Joining puts the open project away, like every other way out of one.
    if (unsavedNeedsAsking && !await askBeforeLeaving()) return null;
    if (opening.value) return null;
    opening.value = true;
    _openProgressWatch?.cancel();
    _openProgressWatch = null;
    openProgress.value = null;
    if (shareJoinOverride?.call(invite) case final stood?) {
      opening.value = false;
      return stood;
    }
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
      _shareEvents = events.stream.listen(handleShareEvent);
      share.begin(ShareRole.guest, project);
      // Everything in a project kept online happened before this machine
      // was in it.
      if (share.online != null) _tellActivity();
      // Joined by the invite that can only look. Said once, and it stays
      // until it is dismissed: every tool on this machine is live, and
      // whatever one does is undone a beat later.
      if (share.online?.look ?? false) {
        _sayShare(l10n.shareLookJoined, sticky: true);
      }
    } else {
      opening.value = false;
    }
    return outcome;
  }

  /// How this machine last said it stands to a project on another Lumit.
  Type? _versionsSaid;

  /// The host agreed: move the open project forward from the older Lumit
  /// that keeps it online to this one. The copy joins by itself when that
  /// is done, as after any time away. What stopped it is said on the line.
  Future<void> moveForward() async {
    final open = project;
    if (open == null || share.forwarding) return;
    share.setForwarding(true);
    var outcome = const BridgeOnlineForward.failed();
    try {
      outcome = shareForwardOverride?.call() ?? await open.shareOnlineForward();
    } catch (_) {
      // Closed while it was being moved.
    }
    if (identical(project, open)) forwardCame(outcome);
  }

  /// Moving the open project forward ended. Public, as [handleShareEvent]
  /// is, so that a test can say how.
  void forwardCame(BridgeOnlineForward outcome) {
    final from = share.versionsOf ?? '';
    share.setForwarding(false);
    if (outcome case BridgeOnlineForward_Update(:final version)) {
      share.setVersions(BridgeShareVersions.update(version: version));
    }
    if (shareForwardText(outcome, from) case final words?) {
      postNotice(words, error: true);
    }
  }

  /// The same for the project [invite] names, before it is joined.
  Future<BridgeOnlineForward> forwardInvite(
      {required String invite, required String name, String? password}) async {
    try {
      return shareForwardOverride?.call() ??
          await bridge_share.onlineForward(
              invite: invite, name: name, password: password);
    } catch (_) {
      return const BridgeOnlineForward.failed();
    }
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
    _activitySoon?.cancel();
    _activitySoon = null;
    _tellUntil = DateTime(0);
    _notesHeard = null;
    _versionsSaid = null;
    // What stayed on the line was about sharing, and that is over.
    if (notice.value?.sticky ?? false) notice.value = null;
    // Ended first, so whoever keeps the counts below hears of it while
    // they are still there to keep.
    share.end();
    shareWatched.clear();
    shareWatchedRecent = const {};
  }

  /// The wait before the activity list is read again.
  Timer? _activitySoon;

  /// How many edits of each sitting this machine has had in front of it, by
  /// who made them and when the sitting began. A sitting reads as new when
  /// any of it was missed, so this is what tells the missed part from what
  /// was watched as it happened. Whoever holds the settings fills it when a
  /// copy is opened and keeps [shareWatchedRecent] when it closes, so a copy
  /// opened again counts only what it missed too.
  final Map<String, int> shareWatched = {};

  /// The part of [shareWatched] worth keeping: the sittings that ended in
  /// the last quarter of an hour, which are the only ones that can still
  /// grow.
  Map<String, int> shareWatchedRecent = const {};

  /// Until when what was missed is still to be said: a few seconds from
  /// joining or coming back, which is when the list catches up.
  DateTime _tellUntil = DateTime(0);

  void _tellActivity() =>
      _tellUntil = DateTime.now().add(const Duration(seconds: 3));

  /// Read who changed what, and the notes people left.
  void _readActivity() {
    _activitySoon?.cancel();
    _activitySoon = null;
    final open = project;
    if (open == null || share.online == null) return;
    final List<BridgeShareSitting> list;
    final List<BridgeShareNote> notes;
    try {
      list = open.shareActivity();
      notes = open.shareNotes();
    } catch (_) {
      // Closed between the event and the read.
      return;
    }
    share.setActivity(list, notes);
    _tellEdits(list);
    // Last, so a note's own words are what the line is left saying.
    _tellNotes(notes);
  }

  /// Just after this machine has joined or come back, say once what
  /// happened while it was not there: before it joined, while its copy was
  /// closed, or while it was away.
  void _tellEdits(List<BridgeShareSitting> list) {
    // What comes in while this machine is away is what it is told of when
    // it is back, so none of it counts as watched yet.
    if (share.away) return;
    final names = <String>[];
    final comps = <String>{};
    final recent = <String, int>{};
    final lately = DateTime.now().millisecondsSinceEpoch ~/ 1000 - 900;
    var edits = 0;
    for (final sitting in list) {
      final key = '${sitting.name}|${sitting.began}';
      final missed =
          sitting.isNew ? sitting.edits - (shareWatched[key] ?? 0) : 0;
      shareWatched[key] = sitting.edits;
      if (sitting.ended >= lately) recent[key] = sitting.edits;
      if (missed <= 0) continue;
      edits += missed;
      if (!names.contains(sitting.name)) names.add(sitting.name);
      comps.addAll([
        for (final comp in sitting.comps)
          if (comp.name.isNotEmpty) comp.name,
      ]);
    }
    shareWatchedRecent = recent;
    if (edits == 0 || DateTime.now().isAfter(_tellUntil)) return;
    _tellUntil = DateTime(0);
    final said = LumitNotice(shareActivityText(names, edits, comps),
        onTap: () => share.showActivity?.call());
    recentNotices.insert(0, said);
    if (recentNotices.length > 20) recentNotices.removeLast();
    // Something that went wrong on the way back stays on the line, and so
    // does anything that stays until it is dismissed. This waits behind it,
    // in the list a press on the line shows.
    if ((notice.value?.error ?? false) || (notice.value?.sticky ?? false)) {
      recentNotices.insert(0, recentNotices.removeAt(1));
    } else {
      notice.value = said;
    }
  }

  /// The notes this machine has been told of since sharing began. Null
  /// until the list is first read, when the ones to tell of are those left
  /// while it was not there.
  Set<String>? _notesHeard;

  /// Say that someone else has left a note: its words when it is one, and
  /// how many when it is several. A press leads to them.
  void _tellNotes(List<BridgeShareNote> notes) {
    final heard = _notesHeard;
    final arrived = [
      for (final note in notes)
        if (!note.mine &&
            (heard == null ? note.isNew : !heard.contains(note.id)))
          note,
    ];
    _notesHeard = {for (final note in notes) note.id};
    if (arrived.isEmpty) return;
    postNotice(
        switch (arrived) {
          [final one] => l10n.shareNoteSaid(
              one.name, one.text.replaceAll(RegExp(r'\s+'), ' ')),
          _ => l10n.shareNotesNew(arrived.length),
        },
        onTap: () => share.showActivity?.call());
  }

  /// Take the note called [id] down, for everyone. False when it was not:
  /// it is not this machine's to take down, or it has gone already.
  bool takeNoteDown(String id) {
    final open = project;
    if (open == null) return false;
    try {
      if (!open.shareNoteTakeDown(id: id)) return false;
    } catch (_) {
      // Closed between the press and the call.
      return false;
    }
    _readActivity();
    return true;
  }

  /// Leave a note in the project kept online, with where this person is:
  /// the composition open, the playhead's frame in it and the layer chosen.
  /// False when it was not taken, as on a machine that can only look.
  bool leaveNote(String text,
      {CompositionReference? comp, int? frame, LayerReference? layer}) {
    final open = project;
    if (open == null) return false;
    try {
      if (!open.shareNote(
          text: text, comp: comp, frame: frame, layer: layer)) {
        return false;
      }
    } catch (_) {
      // Closed between the press and the call.
      return false;
    }
    _readActivity();
    return true;
  }

  /// The person has looked at who changed what and at the notes, so none of
  /// it is new to them any more.
  Future<void> activitySeen() async {
    final open = project;
    if (open == null ||
        !(share.activity.any((sitting) => sitting.isNew) ||
            share.notes.any((note) => note.isNew))) {
      return;
    }
    try {
      await open.shareActivitySeen();
    } catch (_) {
      // Closed meanwhile.
      return;
    }
    if (identical(project, open)) _readActivity();
  }

  /// What the line last said of how the shared project stands.
  LumitNotice? _shareSaid;

  /// Say how the shared project stands. The strip's own button carries the
  /// warning colour for it, so the sentence is drawn as news and not as an
  /// alarm, unless it is an [error]. A press leads to the Shared project
  /// window, which says the same.
  void _sayShare(String message,
      {bool error = false, bool sticky = false, VoidCallback? onTap}) {
    postNotice(message,
        error: error,
        sticky: sticky,
        onTap: onTap ?? () => share.showWindow?.call());
    if (notice.value?.message == message) _shareSaid = notice.value;
  }

  /// The Shared project window has opened. It says how the project stands,
  /// so whatever the line was saying of that has been read.
  void shareWindowOpened() {
    if (_shareSaid != null && identical(notice.value, _shareSaid)) {
      notice.value = null;
    }
    _shareSaid = null;
  }

  /// Something happened to the shared project. Public, as [handleChange]
  /// is, so that a test can say it did.
  void handleShareEvent(BridgeShareEvent event) {
    switch (event) {
      case BridgeShareEvent_People(:final people):
        share.setPeople(people);
      case BridgeShareEvent_Away():
        share.setAway(true);
        // A machine the server takes no edits from gives the line up, and
        // has just been told why. That is the thing to say, and it stands.
        // So does how it stands to a project on another Lumit.
        if (!_quiet && share.writable && share.versions == null) {
          share.online == null
              ? postNotice(l10n.shareAway)
              : _sayShare(l10n.shareOnlineAway);
        }
      case BridgeShareEvent_Back(:final held, :final refused, :final dropped):
        // Whether the engine kept this machine's own project from before
        // the merge, which it does when it held or dropped anything.
        share.readReturnCopies();
        share.setAway(false, conflicts: held);
        _versionsSaid = null;
        // The engine put the host's document in place of this one, so every
        // panel reads again, the way it does when a project is swapped.
        final open = project;
        if (open != null) {
          handleChange(ScopedChange(project: open, items: true, local: false));
        }
        final online = share.online != null;
        if (online) _tellActivity();
        if (refused > 0 && online) {
          // The one thing said here that a person must not miss: it stays
          // until they dismiss it or open the window, where a row says the
          // same in the same words.
          share.setDropped(refused, dropped);
          _sayShare(l10n.shareDropped(refused),
              error: true,
              sticky: true,
              onTap: () => held > 0
                  ? share.showConflicts?.call()
                  : share.showActivity?.call());
        } else if (refused > 0) {
          share.setDropped(refused, dropped);
          _sayShare(l10n.shareBackRefused(refused), error: true, sticky: true);
        } else if (!_quiet) {
          postNotice(online ? l10n.shareOnlineBack : l10n.shareBack);
        }
        if (online) _readActivity();
      case BridgeShareEvent_Writable(:final writable):
        // Said on every return to the server, so only a change is news.
        final was = share.writable;
        share.setWritable(writable);
        if (was && !writable) {
          // Two things stop it: the project too large to store, which the
          // engine has said by now, and the host's Lumit Pro ended.
          _sayShare((share.online?.tooLarge ?? false)
              ? l10n.shareOnlineTooLargeSaid
              : (share.online?.host ?? false)
                  ? l10n.shareOnlineWaitingOwn
                  : l10n.shareOnlineWaiting);
        }
      case BridgeShareEvent_Activity():
        // Said as often as edits go in, which is twenty times a second from
        // someone at work. The list is read at most twice in one.
        _activitySoon ??=
            Timer(const Duration(milliseconds: 500), _readActivity);
      case BridgeShareEvent_Versions(:final versions):
        share.setVersions(versions);
        // It was not the server that was missing, so that is taken back.
        if (notice.value?.message == l10n.shareOnlineAway) notice.value = null;
        // Said once for each way of standing, however often it is found.
        final said = versions.runtimeType;
        if (said == _versionsSaid) break;
        _versionsSaid = said;
        switch (versions) {
          case BridgeShareVersions_Movable():
            // The host is asked, once for this opening of the project.
            share.showForward?.call();
          case BridgeShareVersions_HostFirst():
            _sayShare(l10n.shareVersionsHostFirst(shareThisLumit()));
          case BridgeShareVersions_Update(:final version):
            _sayShare(l10n.shareVersionsUpdate(version),
                sticky: true, onTap: () => share.showUpdate?.call());
        }
      case BridgeShareEvent_LookOnly():
        // Something made on a machine that can only look was taken back.
        // Said then, in the words it was told on joining, so the two read
        // as one fact. The engine says it once for a run of edits.
        _sayShare(l10n.shareLookJoined);
      case BridgeShareEvent_LookInvite():
        // The invite that can only look was turned on or off.
        share.onlineSaid();
      case BridgeShareEvent_TooLarge(:final tooLarge):
        // This machine went to store the project and it did not fit, or
        // now does. Said once while edits still go in. When they stop, that
        // is said as it happens.
        share.onlineSaid();
        if (tooLarge && share.writable) {
          _sayShare(l10n.shareOnlineNearlyLargeSaid);
        } else if (!tooLarge &&
            notice.value?.message == l10n.shareOnlineNearlyLargeSaid) {
          notice.value = null;
        }
      case BridgeShareEvent_Elsewhere():
        postNotice(l10n.shareElsewhere, error: true);
      case BridgeShareEvent_Replaced():
        // The invite this copy finds its project by was replaced while it
        // was away. It stays away until it is given the new one, which the
        // window a press leads to has the place for.
        share.setReplaced();
        _sayShare(l10n.shareReplacedSaid, sticky: true);
      case BridgeShareEvent_Reach(:final reach):
        share.setReach(reach);
      case BridgeShareEvent_Relayed(:final relayed):
        share.setRelayed(relayed);
      case BridgeShareEvent_Footage(:final placed):
        // A footage item is being read from a file that has just arrived,
        // so every panel reads again and the Viewer draws again, the way
        // they do when an item is relinked.
        final open = project;
        if (placed && open != null) {
          handleChange(ScopedChange(project: open, items: true, local: false));
        }
        share.footageChanged();
      case BridgeShareEvent_ExportAsked(:final job, :final from, :final comp):
        share.exportAsked(job, from, comp);
      case BridgeShareEvent_Ended(:final reason):
        // The host of a project kept online is told it has closed like
        // everyone else, and is the one who closed it.
        final own = reason is BridgeShareEnding_Closed &&
            (share.online?.host ?? false);
        // Someone who can only look is turned away when no more of them
        // are taken, or when someone who can edit needs the place.
        final looker = reason is BridgeShareEnding_Full &&
            (share.online?.look ?? false);
        stopSharing();
        if (!own) {
          postNotice(looker ? l10n.shareFullLook : shareEndingText(reason),
              error: reason is! BridgeShareEnding_Closed);
        }
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
  /// share of the whole open that sits behind it; the last stretch is the
  /// first frame, which fills it as it is made ([previewProgress]), and
  /// [previewReady] closes it at 1. Weighted phases, not a timer: the card
  /// would rather move in honest steps than sweep a bar that knows nothing.
  final ValueNotifier<OpenProgress?> openProgress = ValueNotifier(null);

  /// Where the bar stood when the first frame began to report, so the frame
  /// fills what is left of it.
  double _previewFrom = 0;

  /// How long the first frame has been reporting.
  final Stopwatch _firstFrame = Stopwatch();

  /// The first frame saying how far it has got, 0..1.
  void previewProgress(double fraction) {
    final last = openProgress.value;
    if (!opening.value || last == null) return;
    if (!_firstFrame.isRunning) {
      _previewFrom = last.fraction;
      _firstFrame.start();
    }
    // A frame this slow has its own bar in the Viewer, and the shell is more
    // use with that over it than with the card.
    if (_firstFrame.elapsed > const Duration(seconds: 3)) {
      previewReady();
      return;
    }
    _reportOpenProgress(OpenProgress(
        phase: OpenPhase.startingPreview,
        fraction: _previewFrom + (1 - _previewFrom) * fraction));
  }

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

  /// What the card's Cancel button says, or null for the plain word. Set and
  /// let go with [busyCancel].
  final ValueNotifier<String?> busyCancelLabel = ValueNotifier(null);

  /// Run a job with the card up over the shell, the engine's own progress on
  /// its bar and a Cancel that stops it.
  ///
  /// [after] holds the card back for a job that is usually over at once: an
  /// ordinary save of a packed project copies its footage too, and a card that
  /// flashed up on every Ctrl+S of a small one would be noise.
  Future<T> withBusyCard<T>(
    String label,
    Future<T> Function(RustStreamSink<double> progress) start, {
    required VoidCallback cancel,
    String? cancelLabel,
    Duration after = Duration.zero,
  }) async {
    // Set before the card goes up: it keeps whichever bar it opened with.
    busyProgress.value = 0;
    busyCancel.value = cancel;
    busyCancelLabel.value = cancelLabel;
    StreamSubscription<double>? watching;
    Timer? raise;
    try {
      final progress = RustStreamSink<double>();
      // The call is started before the sink is listened to, as an open's is: a
      // sink has no stream until it has been handed to a call.
      final pending = start(progress);
      watching = progress.stream.listen((fraction) {
        if (fraction >= (busyProgress.value ?? 0)) {
          busyProgress.value = fraction;
        }
      });
      raise = Timer(after, () => busy.value = label);
      return await pending;
    } finally {
      raise?.cancel();
      watching?.cancel();
      busy.value = null;
      busyProgress.value = null;
      busyCancel.value = null;
      busyCancelLabel.value = null;
    }
  }

  /// The Viewer has something to show, or there is nothing for it to show —
  /// either way the shell can come out from behind its progress bar.
  ///
  /// Called when the new project's worker has made its first frame or given up
  /// on it (a project whose first render faults must not leave the interface
  /// covered), and by the session restore when it has no frame to ask for.
  void previewReady() {
    if (!opening.value) return;
    _firstFrame
      ..stop()
      ..reset();
    // Full, then gone: the bar reaching its end is what "open" means, and a
    // card that vanished at eighty per cent would read as one that gave up.
    final last = openProgress.value;
    if (last != null) {
      openProgress.value = OpenProgress(phase: last.phase, fraction: 1);
    }
    opening.value = false;
  }

  /// Whether the open in flight is an import, which is what its card's Cancel
  /// button then says.
  bool openIsImport = false;

  /// Whether the open in flight was asked to stop.
  bool _openCancelled = false;

  /// How the open in flight is stopped, or null while it cannot be: before it
  /// has run long enough to be worth stopping, and once the engine's part is
  /// over and only the preview is left to start.
  final ValueNotifier<VoidCallback?> openCancel = ValueNotifier(null);

  /// Ask the engine to stop the open or import in flight. It stops at its
  /// next step and the project on screen stays. One that has already got past
  /// its last step opens all the same.
  void _cancelOpen() {
    _openCancelled = true;
    cancelImport();
  }

  /// Offer Cancel on the opening card after a moment, so the button does not
  /// flash on an open that is over at once. The caller cancels the timer and
  /// clears [openCancel] when the engine has answered.
  Timer _offerOpenCancel() {
    _openCancelled = false;
    return Timer(const Duration(milliseconds: 300),
        () => openCancel.value = _cancelOpen);
  }

  /// [recover] false opens the file as it is, for the recovery dialogue's own
  /// opens, which must not ask the question again.
  ///
  /// [launch] is the open a document on the command line asks for, by a Lumit
  /// started to do nothing else.
  Future<void> openProject(String path,
      {bool recover = true, bool launch = false}) async {
    // One at a time: the change sink below is a single pending field, and two
    // opens in flight would have the second take the first's.
    if (opening.value) return;
    if (unsavedNeedsAsking && !await askBeforeLeaving()) return;
    // Asked again, because another open can have started while the question
    // was up.
    if (opening.value) return;
    opening.value = true;
    openIsImport = false;
    final offer = _offerOpenCancel();
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
    // Anything but opened leaves the previous project loaded, rather than the
    // app being left with none.
    final outcome = await pending;
    offer.cancel();
    openCancel.value = null;
    if (outcome is! BridgeOpened_Opened) {
      openProgress.value = null;
      opening.value = false;
      if (outcome is BridgeOpened_OpenElsewhere) {
        // Another Lumit window has this project, and it has come forward
        // where the system allows. A Lumit started only to open it has
        // nothing else to show, so it goes: opening a project twice from the
        // file manager is one window.
        if (outcome.focused && launch && launchInvite == null) exit(0);
        if (!outcome.focused) postNotice(l10n.projectOpenElsewhere);
      } else if (!_openCancelled) {
        postNotice(l10n.couldNotOpen(path), error: true);
      }
      return;
    }
    final opened = outcome.project;
    // Deliberately still `opening`: the document is in, the picture is not.
    _adopt(opened);
    // A guest's own copy, closed while its host was away, opens still a guest
    // and still looking. What it has found since comes down the events. A
    // copy of a project kept online opens a member of it however it was
    // closed, and looks for Lumit's server.
    var guest = false;
    try {
      guest = opened.shareGuest() || opened.shareOnlineState() != null;
    } catch (_) {
      // Closed again already.
    }
    if (guest) {
      _shareEvents = shareEvents.stream.listen(handleShareEvent);
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
      if (crashed) {
        unawaited(offerRecovery?.call(path));
        return;
      }
    }
    _tellOpened(opened, path);
  }

  /// Run the project-opened script once [opened] is in and its card has
  /// gone, if it is still the open project then.
  void _tellOpened(ProjectReference opened, String path) {
    void check() {
      if (opening.value) return;
      opening.removeListener(check);
      if (identical(project, opened)) {
        runEventScript?.call(ScriptEvent.open, path);
      }
    }

    opening.addListener(check);
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
    openIsImport = true;
    final offer = _offerOpenCancel();
    // Determinate from the first frame and reported as an open is, for
    // [openProject]'s reasons.
    openProgress.value =
        OpenProgress(phase: OpenPhase.readingFile, fraction: 0);
    final progress = RustStreamSink<OpenProgress>();
    final pending = LumitBridgeState.importAeBundle(
        path: target,
        onChangeStream: _changeSink(),
        onProgressStream: progress);
    _openProgressWatch?.cancel();
    _openProgressWatch = progress.stream.listen(_reportOpenProgress);
    final imported = await pending;
    offer.cancel();
    openCancel.value = null;
    if (imported == null && _openCancelled) {
      openProgress.value = null;
      opening.value = false;
      return null;
    }
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
    handleChange(ScopedChange(project: opened, items: true, local: false));
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
  /// Ctrl-Z. The engine reads the files off the interface's thread and files
  /// them together, and a layered document comes in as a composition of its
  /// layers. [bins] puts each path in the project folder of that name, as
  /// `Shoot/Day 1`, which is how an imported folder keeps its shape.
  ///
  /// A big file takes a while to read, so a card goes up over the shell once
  /// it has, with a Cancel that leaves the project as it was. One import at a
  /// time: a second asked for while one runs is refused.
  Future<bool> importFootagePaths(List<String> paths,
      {List<String> bins = const []}) async {
    final project = this.project;
    if (project == null || paths.isEmpty) return false;
    if (_importing || opening.value || busy.value != null) return false;
    _importing = true;
    try {
      final imported = await withBusyCard(
        l10n.importingFiles(paths.length),
        (progress) => project.importFiles(
            paths: paths, bins: bins, onProgress: progress),
        cancel: cancelImport,
        cancelLabel: l10n.cancelImport,
        after: const Duration(milliseconds: 300),
      );
      if (imported.cancelled) return false;
      if (imported.leftOut > 0) {
        postNotice(l10n.importLayersLeftOut(imported.leftOut));
      }
    } finally {
      _importing = false;
    }
    notifyDocumentChanged();
    return true;
  }

  /// An import of files is running.
  bool _importing = false;

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
      arrival:
          Provider.of<LumitUiState>(context, listen: false).streamsArrival,
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

  /// It keeps the line until it is dismissed, and is not pushed off by the
  /// next quiet thing said. For the few a person must not miss.
  final bool sticky;

  /// What a press on its words does, for a notice that leads somewhere.
  final VoidCallback? onTap;

  /// A button beside the words that does the same, for where it leads.
  final String? action;
  const LumitNotice(this.message,
      {this.error = false, this.sticky = false, this.action, this.onTap});
}
