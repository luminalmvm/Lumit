// Lumit's Flutter frontend (K-174, the frontend alternative experiment).
// The engine stays in the Rust crates; this application is the chrome —
// see docs/archive/flutter-port/ for the plan and the parity checklist.

import 'dart:async';
import 'dart:convert';
import 'dart:io' show Directory, File, Platform;
import 'dart:ui' show AppExitResponse;

import 'package:flutter/foundation.dart' show kDebugMode;
import 'package:flutter/gestures.dart' show GestureBinding;
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_rust_bridge/flutter_rust_bridge_for_generated.dart';
import 'package:lumit_flutter/data/expressions_metadata.dart';
import 'package:lumit_flutter/panels/easing_curve.dart' show EasingCurve;
import 'package:lumit_flutter/panels/effect_param_row_frb.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/panels/panels_frb.dart';
import 'package:lumit_flutter/panels/timeline_extras_frb.dart';
import 'package:lumit_flutter/panels/viewer_texture_controller.dart';
import 'package:lumit_flutter/shell/comp_settings_frb.dart';
import 'package:lumit_flutter/shell/precompose_dialog_frb.dart';
import 'package:lumit_flutter/shell/dock_widget.dart';
import 'package:lumit_flutter/shell/about_window_frb.dart';
import 'package:lumit_flutter/shell/first_run_frb.dart';
import 'package:lumit_flutter/shell/fx_console_frb.dart'
    show lastKnownPointerPosition;
import 'package:lumit_flutter/shell/menu_bar_frb.dart';
import 'package:lumit_flutter/shell/project_settings_frb.dart';
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/shell/splash.dart';
import 'package:lumit_flutter/shell/status_line_frb.dart';
import 'package:lumit_flutter/shell/tool_bar_frb.dart';
import 'package:lumit_flutter/shell/welcome_frb.dart';
import 'package:lumit_flutter/src/rust/api/cache.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart' show BridgeNodeRef;
import 'package:lumit_flutter/src/rust/api/import.dart' show BridgeImportReport;
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/shell.dart' show bootLog;
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:lumit_flutter/src/rust/frb_generated.dart';
import 'package:lumit_flutter/state/comp_model.dart';
import 'package:lumit_flutter/state/clipboard.dart';
import 'package:lumit_flutter/state/comp_time.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/dropper.dart';
import 'package:lumit_flutter/state/keymap.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart';
import 'package:lumit_flutter/state/animated_mask_paths.dart';
import 'package:lumit_flutter/state/layer_bounds.dart';
import 'package:lumit_flutter/state/preview_progress.dart';
import 'package:lumit_flutter/state/render_timings.dart';
import 'package:lumit_flutter/state/settings.dart';
import 'package:lumit_flutter/state/install_site.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:lumit_flutter/state/updates.dart';
import 'package:lumit_flutter/state/viewer_view.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/ui_scale.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

class StackTraceEntry {
  StackTrace trace;
  String name;
  late DateTime time;
  late Duration duration;
  bool async;

  StackTraceEntry(
      {required this.name,
      required this.trace,
      required this.duration,
      required this.async}) {
    time = DateTime.now();
  }
}

class FunctionCallStats {
  int numCalls = 0;
  Duration totalTime = Duration.zero;
  Duration lastTime = Duration.zero;

  double get averageMs =>
      totalTime.inMilliseconds.toDouble() / numCalls.toDouble();
}

class LumitDebugUI {
  List<StackTraceEntry> rustCalls = List.empty(growable: true);
  Map<String, FunctionCallStats> stats = {};

  StreamController onChange = StreamController.broadcast();

  void addStackTrace(StackTraceEntry trace) {
    rustCalls.insert(0, trace);

    const maxLen = 100;

    if (stats.containsKey(trace.name) == false) {
      stats[trace.name] = FunctionCallStats();
    }
    var stat = stats[trace.name]!;

    stat.numCalls += 1;
    stat.totalTime += trace.duration;
    stat.lastTime = trace.duration;

    if (rustCalls.length > maxLen) {
      rustCalls = rustCalls.sublist(0, maxLen);
    }

    onChange.add(null);
  }

  void clear() {
    rustCalls.clear();
    onChange.add(null);
  }
}

LumitDebugUI debugInfo = LumitDebugUI();

/// Traces every call that crosses into Rust, so the frb seam can be watched
/// while it is being built out. `debugPrint` rather than `print`: it compiles
/// away in release, where a log per bridge call would be far too costly.
class CustomHandler extends BaseHandler {
  @override
  Future<S> executeNormal<S, E extends Object>(NormalTask<S, E> task) async {
    var stack = StackTrace.current;

    var str = stack.toString();
    var lines = str.split("\n");

    var target = lines.elementAtOrNull(2);
    var split = target?.split(" ");

    final start = DateTime.now();
    final result = await super.executeNormal(task);
    final end = DateTime.now();

    var duration = end.difference(start);

    if (split != null) {
      final item = split.elementAtOrNull(split.length - 2);
      debugInfo.addStackTrace(StackTraceEntry(
          name: item!, trace: stack, duration: duration, async: true));
    }

    return result;
  }

  @override
  S executeSync<S, E extends Object, WireSyncType>(
      SyncTask<S, E, WireSyncType> task) {
    var stack = StackTrace.current;

    var str = stack.toString();
    var lines = str.split("\n");

    var target = lines.elementAtOrNull(2);
    var split = target?.split(" ");

    final start = DateTime.now();
    final result = super.executeSync(task);
    final end = DateTime.now();

    var duration = end.difference(start);

    if (split != null) {
      final item = split.elementAtOrNull(split.length - 2);
      debugInfo.addStackTrace(StackTraceEntry(
          name: item!, trace: stack, duration: duration, async: false));
    }

    return result;
  }
}

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
String? projectPathFromArgs(List<String> args) {
  for (final a in args) {
    if (a.toLowerCase().endsWith('.lum') && File(a).existsSync()) return a;
  }
  return null;
}

Future<void> main(List<String> args) async {
  WidgetsFlutterBinding.ensureInitialized();

  // Sweep up after an update before anything else happens (K-297): delete the
  // version we have just replaced, now that nothing is holding its files, and
  // put it back if a swap was cut in half. Never throws and never blocks — a
  // tidying problem is not a reason for an editor not to open.
  tidyAfterUpdate(InstallSite.detect());

  // The call tracer takes StackTrace.current on every bridge call, which is
  // debugging money a release build must not spend.
  await BridgeLib.init(handler: kDebugMode ? CustomHandler() : null);
  await ExpressionsMetadata.load();
  await ExpressionTextEditingController.initSyntaxHighlighting();
  final state = LumitState();
  // Start with an empty project rather than nothing at all. Every document
  // command — import, new composition, save — is disabled while there is no
  // project, so booting without one left the whole File and Composition menu
  // dead and no way to make it live: the first thing a user does needs
  // something to do it *to*.
  state.newProject();
  // A document on the command line opens over the empty project. On failure
  // openProject posts its notice and the empty project stands — the same
  // degraded-but-alive behaviour as a failed File → Open.
  final fromArgs = projectPathFromArgs(args);
  if (fromArgs != null) state.openProject(fromArgs);
  // Somebody who double-clicked a `.lum` has already answered the welcome
  // screen's question, so it is not put to them (K-464).
  runApp(LumitAppNew(state, LumitUiState(state), welcome: fromArgs == null));
}

class LumitState extends ChangeNotifier {
  ProjectReference? project;

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

  void postNotice(String message, {bool error = false}) =>
      notice.value = LumitNotice(message, error: error);

  void newProject() {
    _adopt(LumitBridgeState.newProject(onChangeStream: _changeSink()));
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

  /// The line on the card shown over the shell while some other seconds-long
  /// job runs, or null when none is. Beat detection is the first of them.
  ///
  /// Separate from [opening] because the two say different things: [opening] is
  /// a document being swapped underneath the panels, this is the document
  /// standing still while something works on it. Set it through `showBusyWhile`
  /// (shell/splash.dart) so the card cannot be left up by a job that failed.
  final ValueNotifier<String?> busy = ValueNotifier(null);

  /// The Viewer has something to show, or there is nothing for it to show —
  /// either way the shell can come out from behind its progress bar.
  ///
  /// Called on the first sign of life from the new project's worker (any reply
  /// at all, not only a frame: a project whose first render faults must not
  /// leave the interface covered), and by the session restore when it fronts no
  /// composition, which is a project with no picture to wait for.
  void previewReady() {
    if (opening.value) opening.value = false;
  }

  Future<void> openProject(String path) async {
    // One at a time: the change sink below is a single pending field, and two
    // opens in flight would have the second take the first's.
    if (opening.value) return;
    opening.value = true;
    // Null means the file would not open; the previous project stays loaded
    // rather than the app being left with none.
    final opened = await LumitBridgeState.openProject(
        path: path, onChangeStream: _changeSink());
    if (opened == null) {
      postNotice(l10n.couldNotOpen(path), error: true);
      opening.value = false;
      return;
    }
    // Deliberately still `opening`: the document is in, the picture is not.
    _adopt(opened);
  }

  /// Import an After Effects project and make it the open one (docs/11) —
  /// either front door (K-418): the `.aep` itself, or a Lumit Bridge bundle.
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
    final imported = await LumitBridgeState.importAeBundle(
        path: target, onChangeStream: _changeSink());
    if (imported == null) {
      // Three misses, three answers. An `.aep` the parser could not read is
      // the one K-418 made possible and the one worth being calm about: a
      // newer After Effects may store something this build has not met, and
      // the Bridge route reads it in full. A *folder* holding an `.aep` is the
      // older mistake — the bundle picker asked for a folder and the user
      // reasonably pointed it at the project's own — and still teaches the
      // route. Anything else is simply not a bundle.
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
    project = opened;
    // The comp list is cached per document (K-184) and invalidated when the
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
    notifyListeners();
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
  /// shared by the keyboard chords and the Composition menu (K-197, docs/04
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
  Future<bool> importFootagePaths(List<String> paths) async {
    final project = this.project;
    if (project == null || paths.isEmpty) return false;
    for (final path in paths) {
      project.importFootage(path: path);
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

  void handleChange(ScopedChange event) {
    // The item tree changed shape: the cached comp list is stale.
    if (event.items || event.item != null) _compsCache = null;

    _onChange.add(event);

    // A change that names a subtree is that subtree's business: the comp read
    // model and ProjectItemBuilder subscribe to the stream themselves.
    if (event.layer != null || event.item != null) return;

    _compsCache = null;
    // Nothing narrower to aim at — whoever listens to LumitState rebuilds.
    notifyListeners();
  }

  /// Every composition in the project with its name, folders walked — cached
  /// so the comp tabs cost no bridge calls per rebuild (K-184). Invalidated
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

class LumitUiState extends ChangeNotifier {
  /// Everything that outlives the session: the panel layout, the appearance,
  /// UI scale, tooltips, autosave and export defaults.
  ///
  /// This is the same [Workspace] the shell has always used, loaded from disk
  /// on construction — the port briefly kept its own copies of the layout and
  /// the colour scheme here instead, which is why arrangements stopped
  /// surviving a restart and the Settings window's scale slider moved nothing.
  final Workspace workspace;

  /// The keyboard map every shortcut is looked up in (docs/07 §15, K-199).
  late final KeymapState keymap;

  /// Whether there is a newer Lumit, and fetching it (K-296).
  ///
  /// One for the session, here, because the Help menu and Settings ▸ General
  /// are two views of the same check and neither owns it. The version is passed
  /// as a function, not a string: it comes over the bridge, and a widget test
  /// that builds this state must not call the engine merely by existing.
  late final UpdateService updates =
      UpdateService(currentVersion: () => versionFromBootLine(lumitVersion()));

  /// How big each layer's content is, for the Viewer's boxes and hit-testing
  /// (K-217). Held here because the answer is the document's, not a panel's,
  /// and probing a clip is disk work that must happen once rather than per
  /// Viewer rebuild.
  final LayerBoundsCache layerBounds = LayerBoundsCache();

  /// Where a keyed mask's shape actually is at the frame on screen (K-342), so
  /// the Viewer's wireframe follows an animated path instead of the still one
  /// the mask still carries. Held against the document and the playhead, so a
  /// hover asks the engine nothing.
  final AnimatedMaskPaths animatedMaskPaths = AnimatedMaskPaths();

  /// Which tool the toolbar has armed (docs/07 §1.7, K-216).
  ///
  /// Session state at the shell level, like the dropper below it and for the
  /// same reason: the tool is picked in one place and read in another, and no
  /// panel should have to be mounted for either.
  final ToolsState tools = ToolsState();

  DockSplit get split => workspace.dock;
  ValueNotifier<Panel?> activePanel = ValueNotifier(null);

  /// Move the focus ring on by [by] panels in the arrangement's own order —
  /// `Ctrl+F6` forwards, `Ctrl+Shift+F6` back (docs/07 §15, "Panels").
  ///
  /// The arrangement's order, not the enum's: what the ring walks is what is on
  /// screen, left to right and top to bottom as the tree visits it, so a panel
  /// dropped from the workspace is simply not in the cycle. A panel sitting
  /// behind a tab is *brought to the front* as it is reached, because a focus
  /// ring on something nobody can see is a keystroke that appears to have done
  /// nothing.
  ///
  /// Answers whether it moved, so an arrangement with nothing in it leaves the
  /// chord to whatever else might want it.
  bool cyclePanelFocus(int by) {
    final panels = panelsIn(split);
    if (panels.isEmpty) return false;
    final current = activePanel.value;
    final at = current == null ? -1 : panels.indexOf(current);
    // Nothing focused yet: the first panel is where a cycle begins, whichever
    // way it was asked to go.
    final next = at < 0 ? panels.first : panels[(at + by) % panels.length];
    activatePanelTab(split, next);
    activePanel.value = next;
    // Which tab a group fronts is part of the arrangement, and the arrangement
    // persists — `touch` both redraws the dock and writes it down.
    workspace.touch();
    return true;
  }

  /// Bumped when `Ctrl+F` asks the focused panel to put the cursor in its
  /// search box (docs/07 §15).
  ///
  /// A notifier for the same reason as [togglePlayRequest]: the field belongs
  /// to whichever panel is focused, and the shell has no business reaching into
  /// one. Each panel with a search box listens and answers only when it is the
  /// focused one, so one request can never focus two fields.
  final ValueNotifier<int> panelSearchRequest = ValueNotifier(0);

  /// Ask the focused panel for its search box, and say whether there is one to
  /// ask for. Only two panels have one (docs/07 §15); anywhere else the chord
  /// is left alone rather than swallowed.
  bool requestPanelSearch() {
    final panel = activePanel.value;
    if (panel != Panel.project && panel != Panel.effectsAndPresets) {
      return false;
    }
    panelSearchRequest.value++;
    return true;
  }

  /// Whether [panel] is the one a [panelSearchRequest] is meant for.
  bool searchRequestIsFor(Panel panel) => activePanel.value == panel;

  /// A finer selection's claim on Delete (K-234), set by the Timeline while it
  /// is mounted and cleared when it goes.
  ///
  /// The shell's Delete removes the selected *layers*, which is only the right
  /// answer when nothing smaller is selected: with a mask row picked, Delete
  /// means that mask, and deleting the layer it sits on instead is the opposite
  /// of what was asked. The shell asks this first and stands down when it
  /// returns true. A callback rather than a race between key handlers: every
  /// hardware-keyboard handler runs on every key, so a panel cannot claim a
  /// chord simply by handling it.
  bool Function()? deleteClaim;

  /// The same claim, for Copy and Paste (K-300). The Timeline sets these while
  /// it is mounted: with keyframes selected, `Mod+C` means those keyframes, and
  /// `Mod+V` puts them back — the layer clipboard is what the chord falls
  /// through to. Each returns whether it took the chord.
  bool Function()? copyClaim;
  bool Function()? pasteClaim;

  /// Where the Easing panel sends a shape (K-349), published by the Timeline
  /// while it can take one and null when it cannot.
  ///
  /// The same claim idea as the three above, but a notifier rather than a bare
  /// field, because this one is *read to draw with*: the panel is persistent, so
  /// it must grey its Apply the moment there is nowhere to send a shape —
  /// no Timeline on screen, or a graph showing the speed lens, where a curve
  /// drawn against value travel does not belong (K-348). A bare field would
  /// leave the panel showing a live button until something else happened to
  /// rebuild it.
  ///
  /// The keyframe selection itself stays the Timeline's and is never published:
  /// the panel sends a shape and is told nothing about what it landed on.
  final ValueNotifier<ValueChanged<EasingCurve>?> easingApply =
      ValueNotifier(null);

  /// The appearance the shell is drawing in.
  ///
  /// Scheme and shape are held rather than the built theme, because the theme is
  /// derived from them — keeping the composed object as the source of truth
  /// would make "what did the user choose?" a question you answer by comparing
  /// colours.
  LumitColorScheme get scheme => workspace.colorScheme;
  ThemeShape get shape => workspace.themeShape;
  LumitTheme get theme => workspace.theme;

  /// Bumped when something outside the Viewer asks the transport to start or
  /// stop — the space bar, or a command.
  ///
  /// A notifier rather than a direct call because the ticker that runs playback
  /// belongs to the Viewer's own state: the shell should not have to reach into
  /// a panel, and the Viewer should not have to be mounted for the key to be
  /// harmless.
  final ValueNotifier<int> togglePlayRequest = ValueNotifier(0);

  void requestTogglePlay() => togglePlayRequest.value++;

  /// Look for a newer Lumit on launch, if that is switched on and it has been
  /// a day since the last look (K-296).
  ///
  /// Only ever a *look*: what it finds ends up as the wording of the Help menu
  /// row, and downloading anything still waits for a click. Failure is silent —
  /// a machine with no network has not done anything wrong, and an editor that
  /// opened with a complaint about the internet would be insufferable.
  Future<void> maybeCheckForUpdates() async {
    if (!workspace.autoUpdate) return;
    // Never under `flutter test`: a suite that mounts the shell would otherwise
    // reach the network, which is slow, flaky, and none of a test's business.
    if (Platform.environment.containsKey('FLUTTER_TEST')) return;
    if (!updates.dueForCheck(workspace.lastUpdateCheckMs)) return;
    await updates.check();
    workspace.rememberUpdateCheck(DateTime.now().millisecondsSinceEpoch);
  }

  /// Bumped when `Ctrl+Shift+P` asks for the command palette.
  ///
  /// A notifier for the same reason as [togglePlayRequest]: the palette's list
  /// of commands is the menu bar's, declared beside the menu items so the two
  /// cannot drift into different ideas of what "New composition" does. The
  /// shortcut asks for the palette rather than building a second list of its
  /// own — which would be exactly the drift that note warns about.
  final ValueNotifier<int> paletteRequest = ValueNotifier(0);

  void requestPalette() => paletteRequest.value++;

  /// Bumped when `Ctrl+Space` asks for the FX console (K-324). Its effects,
  /// comps and radial entries are the menu bar's, for the same reason the
  /// palette's commands are.
  final ValueNotifier<int> consoleRequest = ValueNotifier(0);

  void requestConsole() => consoleRequest.value++;

  /// A property row the Timeline has been asked to show — the layer and one of
  /// the `reveal.*` actions (docs/07 §4.3's P/S/R/T/A family). Set by the FX
  /// console's Keyframe ring (K-326) after it plants a key, so the key just
  /// made is on screen. The Timeline listens and *ensures* the row is open —
  /// no toggle, unlike the reveal keys, because asking to see a row twice
  /// should never hide it.
  final ValueNotifier<(UuidValue, String)?> revealPropertyRequest =
      ValueNotifier(null);

  void requestRevealProperty(UuidValue layer, String action) =>
      revealPropertyRequest.value = (layer, action);

  /// **Which property rows the Timeline has picked**, as their paths, published
  /// so the Viewer can answer the same question (K-341).
  ///
  /// A property belongs to a layer, so picking one is saying which layer is
  /// being worked on — and the Viewer should outline that layer and its masks
  /// exactly as it does for a layer picked on its own row. It also tells the
  /// Viewer which mask's shape is being edited: with a mask's **Path** row
  /// picked, that mask is the one whose points are offered for dragging.
  final ValueNotifier<List<String>> selectedProperties =
      ValueNotifier(const []);

  /// The other direction: something outside the Timeline asking it to pick a
  /// property row. Set by the Viewer when a mask path with keyframes is
  /// dragged, so the row whose key just moved is the row on screen.
  final ValueNotifier<String?> selectPropertyRequest = ValueNotifier(null);

  void requestSelectProperty(String path) => selectPropertyRequest.value = path;

  /// The Project panel's picked item — its selection anchor, published by the
  /// panel on every click (K-327). The full selection stays the panel's own;
  /// this is the one item the FX console acts on, so a Ctrl+Space over the
  /// Project panel offers "add this to the comp" rather than the new-layer
  /// ring it used to fall through to. Null with nothing picked there.
  final ValueNotifier<ItemReference?> selectedProjectItem = ValueNotifier(null);

  /// Bumped each time a rendered frame reaches the Viewer, on any of the three
  /// transports. Watched by anything that redraws when the picture does — the
  /// Timeline's cache bar, the Scopes panel.
  final ValueNotifier<int> frameArrived = ValueNotifier(0);

  /// Bumped when the engine banks a frame in the background (the idle cache
  /// fill). Its own notifier, not [frameArrived]: the picture did not change,
  /// so nothing that re-renders on a new picture (the Scopes) should stir —
  /// only the cache bar, which listens to both.
  final ValueNotifier<int> cacheChanged = ValueNotifier(0);

  /// Bumped when a Camera track analysis lands a solve (K-430). Its own
  /// notifier because a solve moves neither the document's revision nor the
  /// playhead — and those two are exactly what the Viewer's point cloud is
  /// keyed by, so without this it had no reason to ask again and the dots did
  /// not appear until the frame changed.
  final ValueNotifier<int> solveLanded = ValueNotifier(0);

  /// The preview tier the last frame was made at: 1 Full, 2 Half, 3 Third,
  /// 4 Quarter (K-030/K-171).
  ///
  /// Carried on the frame rather than asked for. The Viewer shows the tier in
  /// two places, and each of them asked the engine in its `build()` — two calls
  /// across the boundary for each frame of playback, ~48 a second at 24 fps,
  /// for a number that only a new frame can change.
  final ValueNotifier<int> previewTier = ValueNotifier(1);

  /// How far the frame the Viewer is waiting for has got, when that is worth
  /// drawing (docs/07 §2.5). Fed from the worker stream below; the Viewer's
  /// progress bar listens to it and nothing else does.
  final PreviewProgressTracker previewProgress = PreviewProgressTracker();

  /// The last measured frame's per-layer and per-effect render times
  /// (docs/13 §7.1). Empty — and the engine not measuring — until a column or
  /// a panel that shows the numbers asks for them.
  ///
  /// Switching it on asks for the frame under the playhead again, because
  /// numbers only exist for a frame the engine actually composites: without
  /// this the column sat empty until something else happened to want a render,
  /// which on a comp the idle fill had already made could be for ever.
  late final RenderTimings renderTimings = RenderTimings(
    onMeasuringStarted: requestFrame,
    // An engine that refuses the switch says so in the status line rather than
    // leaving a lit stopwatch over a column that will never fill.
    onEngineError: (error) => _app.postNotice(
      l10n.couldNotMeasureRenderTimes('$error'),
      error: true,
    ),
  );

  /// Whether the engine is playing.
  ///
  /// Mirrored, not decided: it goes true when [play] is called and false when
  /// the engine says playback ended or the user stops it. The transport reads it
  /// to know which button to draw.
  final ValueNotifier<bool> playing = ValueNotifier(false);

  /// Start playing the fronted composition from the playhead.
  ///
  /// Everything about *how* playback runs — which frame is next, whether the
  /// clock has moved on, when to give up a tier — belongs to the engine
  /// (K-181). This says go, and [_arrived] follows the frames back.
  void play() {
    final comp = selectedComp;
    if (comp == null) return;
    // The work area is the span being worked on, so it is the span playback
    // runs round: reaching its end goes back to its start and carries on,
    // rather than playing out to the end of the comp and stopping. Read once
    // here rather than per frame — it cannot change while the transport is
    // running, and [_arrived] fires at the comp's rate.
    final set = comp.getWorkArea();
    _loop = set == null
        ? null
        : (
            start: comp.frameAtTime(time: set.inPoint),
            end: comp.frameAtTime(time: set.outPoint)
          );
    _playedFrom = playheadFrame.value;
    // Whatever the scrub before this was waiting for, it is not what the user
    // is watching now: playback draws no progress bar (docs/07 §2.5), and one
    // left standing from the frame that started the run would be the only bar
    // that ever appeared during playback.
    previewProgress.stop();
    _playFrom(comp, playheadFrame.value);
    playing.value = true;
  }

  /// Where the playhead stood when [play] was called, so stopping can put it
  /// back (K-254). Null when nothing is playing.
  ///
  /// Held here rather than read off the comp because it is a fact about *this
  /// run of the transport*, not about the document: it must survive the frames
  /// arriving and moving the playhead, and it must be forgotten the moment
  /// playback ends however it ends.
  int? _playedFrom;

  /// The work area playback loops round, or null when the comp has not been
  /// narrowed — in which case playback ends at the end, as it always did.
  ({int start, int end})? _loop;

  void _playFrom(CompositionReference comp, int frame) => comp.play(
        from: BigInt.from(frame),
        scale: viewerScale,
        mode: workspace.performance.playback == PlaybackMode.adaptive
            ? BridgePlaybackMode.adaptive
            : BridgePlaybackMode.everyFrame,
      );

  /// Stop the transport, and — unless the user is taking hold of the playhead
  /// themselves — put the playhead back where play started (K-254).
  ///
  /// Returning is the default because playback is a *preview*: you park the
  /// playhead where you are working, watch it run, and expect to still be where
  /// you were when it stops. Somebody who wants the playhead to stay where the
  /// picture stopped ticks Settings ▸ Interface ▸ Editing.
  ///
  /// `restorePlayhead: false` is for the one case where returning would fight
  /// the user: scrubbing the ruler stops playback *in order to* move the
  /// playhead, so putting it back would undo the very gesture that stopped it.
  void stopPlayback({bool restorePlayhead = true}) {
    playing.value = false;
    _loop = null;
    selectedComp?.stopPlayback();
    _returnPlayhead(restore: restorePlayhead);
  }

  /// The half of stopping that moves the playhead — shared by the user's stop
  /// and by playback running off the end on its own, because "where am I when
  /// it stops" should not depend on *why* it stopped.
  void _returnPlayhead({bool restore = true}) {
    final from = _playedFrom;
    _playedFrom = null;
    if (!restore || from == null) return;
    if (workspace.interface.playheadStaysOnStop) return;
    // Setting the notifier is the whole of it: the Viewer listens and asks the
    // engine for the frame there, exactly as it does for any other move.
    playheadFrame.value = from;
  }

  /// Ask for the frame under the playhead as the document now stands.
  ///
  /// Called when the playhead moves or an edit lands — both are *facts* the
  /// engine is told, not requests the frontend schedules. The worker coalesces
  /// whatever piles up behind a render in flight (`drain_to_newest`), which is
  /// why this can be called freely and needs no in-flight bookkeeping here.
  /// Ignored during playback, where the engine is already choosing frames.
  void requestFrame() {
    if (playing.value) return;
    final comp = selectedComp;
    if (comp == null) return;
    try {
      comp.renderFrame(
        frame: BigInt.from(playheadFrame.value),
        scale: viewerScale,
        mode: workspace.performance.playback == PlaybackMode.adaptive
            ? BridgePlaybackMode.adaptive
            : BridgePlaybackMode.everyFrame,
      );
    } catch (_) {
      // No worker yet, or a composition that has gone away. The next playhead
      // move or edit asks again; there is nothing to recover here.
    }
  }

  /// A frame arrived. While playing, the picture leads and the playhead follows
  /// it — that is what makes the transport show the frame actually on screen
  /// rather than the one the engine was asked for. Paused, the playhead is the
  /// user's and is left alone.
  void _arrived(int frame) {
    frameArrived.value++;
    if (!playing.value) return;
    if (playheadFrame.value != frame) playheadFrame.value = frame;
    // Round the work area: the frame at its end is shown, then playback starts
    // again from its start. Restarted through `play` rather than by moving the
    // playhead, because the sound and the scheduler's clock both take their
    // baseline from the frame play was asked for.
    final loop = _loop;
    final comp = selectedComp;
    if (loop != null && comp != null && frame >= loop.end) {
      playheadFrame.value = loop.start;
      _playFrom(comp, loop.start);
    }
  }

  /// Move the playhead because the user is taking hold of it — a drag on the
  /// time ruler, a click in the lane area (K-254).
  ///
  /// Different from setting [playheadFrame] directly in one way that matters:
  /// it **stops the transport first**. Scrubbing against running playback was
  /// unwinnable — the engine hands back a frame every tick and each one moved
  /// the playhead straight back off the pointer — so taking hold of it means
  /// taking it off the transport. The playhead does *not* return to where play
  /// started here: the point of the gesture is to end up somewhere else.
  void scrubTo(int frame) {
    if (playing.value) stopPlayback(restorePlayhead: false);
    playheadFrame.value = frame;
  }

  /// Move the playhead by `delta` frames, clamped to the fronted composition.
  void stepFrame(int delta) {
    final comp = selectedComp;
    if (comp == null) return;
    final last = comp.durationFrames() - 1;
    playheadFrame.value =
        (playheadFrame.value + delta).clamp(0, last < 0 ? 0 : last);
  }

  /// The locale the interface is currently drawn in — the saved choice, or the
  /// machine's own language when nothing has been chosen.
  Locale get locale {
    final saved = workspace.interface.language;
    return saved == null ? systemLocale() : localeFromTag(saved);
  }

  /// Point `t` at the current language. Cheap and idempotent, which is why it
  /// can hang off every workspace change rather than needing to know which
  /// setting moved.
  void _applyLanguage() => useLocale(locale);

  /// Settings → Interface → Language. Null means follow the machine.
  void setLanguage(String? tag) {
    workspace.interface.language = tag;
    workspace.settingsChanged();
  }

  /// Put the panels back where they started (Window → Reset workspace).
  void resetLayout() => workspace.resetWorkspaceLayout();

  /// Remember a layout the user changed by dragging a panel — app-wide, and
  /// against the open project, which is what makes two projects able to be
  /// arranged differently (K-245).
  void saveLayout() {
    workspace.save();
    rememberSession();
  }

  void setScheme(LumitColorScheme next) => workspace.setScheme(next);

  void setShape(ThemeShape next) => workspace.setShape(next);

  CompositionReference? _selectedComp;
  CompositionReference? get selectedComp => _selectedComp;

  /// The fronted comp as the panels draw it (K-184) — refreshed by one bridge
  /// call when the engine reports a change, read by everything else for free.
  final CompModel model = CompModel();

  ViewerTextureController controller = ViewerTextureController();

  /// The platform texture the Viewer draws — the only frame transport (K-183):
  /// every frame arrives as a GPU handle, never as pixels. Null before the
  /// first registration.
  ValueNotifier<int?> viewerFrameid = ValueNotifier(null);

  /// The layer everything single-layer works on: Effect controls, the keyboard
  /// commands, the Timeline's fold-out. The *primary* of the selection below.
  ValueNotifier<LayerReference?> selectedLayer = ValueNotifier(null);

  /// Which box the Graph panel has picked (K-471), or null for none.
  ///
  /// Session state at the shell level for the same reason the armed tool is:
  /// it is set in one panel and read in another — the Node panel draws the
  /// picked box's parameter rows — and neither should have to be mounted for
  /// the other to work. An *effect* box also fronts itself in the ordinary
  /// effect selection (K-300); this notifier is what carries the boxes that
  /// selection cannot name, the drivers among them.
  final ValueNotifier<BridgeNodeRef?> graphNode = ValueNotifier(null);

  /// What Copy put down, for Paste to pick up (K-275). One tray for the
  /// session, shared by the Edit menu and the panels.
  ///
  /// Read directly; **written through the two methods below**, because Paste is
  /// greyed out while it is empty and a menu that never hears about the copy
  /// stays greyed until something else happens to repaint it. That is exactly
  /// how it behaved before those methods existed.
  final LumitClipboard clipboard = LumitClipboard();

  /// Copy a layer, and tell the interface so Paste ungreys.
  ///
  /// **Mirrored to the system clipboard** (K-302): a copy that leaves no trace
  /// anywhere the machine can see reads exactly like a copy that did nothing —
  /// paste into a text editor and nothing arrives. The document is the text.
  void copyLayerToClipboard(String text) {
    clipboard.putLayer(text);
    Clipboard.setData(ClipboardData(text: text));
    notifyListeners();
  }

  /// Copy one effect or a whole stack, same repaint, same mirror.
  void copyEffectsToClipboard(String text) {
    clipboard.putEffects(text);
    Clipboard.setData(ClipboardData(text: text));
    notifyListeners();
  }

  /// Take a Lumit document off the **system** clipboard into the tray, if
  /// there is one there and the tray has nothing of its own (K-302).
  ///
  /// This is how a copy made in another Lumit window arrives, and how a paste
  /// still works after something else on the machine has been copied in
  /// between. Ordinary text is left alone — [lumitDocumentKind] only answers
  /// for the two shapes the engine's paste calls accept.
  Future<bool> adoptSystemClipboard() async {
    final text = (await Clipboard.getData(Clipboard.kTextPlain))?.text;
    if (text == null) return false;
    if (text == clipboard.text) return !clipboard.isEmpty;
    switch (lumitDocumentKind(text)) {
      case ClipboardKind.layer:
        clipboard.putLayer(text);
      case ClipboardKind.effects:
        clipboard.putEffects(text);
      case null:
        return false;
    }
    notifyListeners();
    return true;
  }

  /// The whole selection, primary first (K-217).
  ///
  /// Kept beside [selectedLayer] rather than replacing it, because almost
  /// everything in the application acts on one layer and reads it directly —
  /// and a second notifier is cheaper than teaching forty call sites to take
  /// the first element of a list. The two are held in step by [_syncSelection]:
  /// setting [selectedLayer] on its own (which the Timeline and the tests do)
  /// makes that layer the entire selection, which is exactly what clicking one
  /// row means.
  final ValueNotifier<List<LayerReference>> selectedLayers =
      ValueNotifier(const []);

  /// The selection as ids, for a "is this one selected?" test that does not
  /// walk the list per layer per paint.
  Set<UuidValue> get selectedLayerIds =>
      {for (final layer in selectedLayers.value) layer.internallayerId};

  /// Replace the selection. The first entry becomes [selectedLayer].
  void setSelection(List<LayerReference> layers) {
    selectedLayers.value = List.unmodifiable(layers);
    selectedLayer.value = layers.isEmpty ? null : layers.first;
    // An effect belongs to a layer, so picking a different layer cannot leave
    // the old layer's effects picked (K-300) — Copy would then act on something
    // no longer on screen.
    clearEffectSelection();
  }

  /// The effects picked out of one layer's stack (K-300), as instance ids in
  /// **stack order** — what Copy and Cut act on when it is not empty.
  ///
  /// Held here rather than in either panel because an effect is picked in two
  /// places — the Effect controls panel's heading and the Timeline fold-out's
  /// row — and one selection shown in both is what makes those two places one
  /// interface rather than two. [selectedEffectsLayer] is the layer they are
  /// on: the effect ids alone name nothing the engine can find.
  final ValueNotifier<List<UuidValue>> selectedEffects =
      ValueNotifier(const []);
  LayerReference? selectedEffectsLayer;

  /// Replace the effect selection outright — what the Timeline hands over,
  /// having already applied the click rules to its own rows.
  void setEffectSelection(LayerReference layer, List<UuidValue> effects) {
    if (effects.isEmpty) {
      clearEffectSelection();
      return;
    }
    selectedEffectsLayer = layer;
    selectedEffects.value = List.unmodifiable(effects);
    notifyListeners();
  }

  /// Pick [id] by click: plain replaces, Ctrl toggles, Shift extends the run
  /// along [order] (the layer's stack, top to bottom) — the same three rules a
  /// layer row and a property row follow, because a selection that behaved one
  /// way here and another there would be two selections to learn.
  void pickEffect(
    LayerReference layer,
    UuidValue id, {
    required List<UuidValue> order,
  }) {
    final keys = HardwareKeyboard.instance;
    final held = selectedEffectsLayer?.internallayerId == layer.internallayerId
        ? [...selectedEffects.value]
        : <UuidValue>[];
    if (keys.isControlPressed || keys.isMetaPressed) {
      if (!held.remove(id)) held.add(id);
    } else if (keys.isShiftPressed && held.isNotEmpty) {
      final a = order.indexOf(held.last);
      final b = order.indexOf(id);
      if (a < 0 || b < 0) {
        if (!held.contains(id)) held.add(id);
      } else {
        for (var i = a < b ? a : b; i <= (a < b ? b : a); i++) {
          if (!held.contains(order[i])) held.add(order[i]);
        }
      }
    } else {
      held
        ..clear()
        ..add(id);
    }
    setEffectSelection(layer, held);
  }

  /// What **Copy effect** on [id]'s heading takes: the whole picked run when
  /// this effect is part of it, else just this one (K-300). Right-clicking a
  /// heading outside the selection copies what was right-clicked, which is what
  /// every list in the application does.
  List<UuidValue> effectsToCopy(LayerReference layer, UuidValue id) =>
      selectedEffectsLayer?.internallayerId == layer.internallayerId &&
              selectedEffects.value.contains(id)
          ? selectedEffects.value
          : [id];

  /// Nothing picked out of any stack — a layer chosen, a parameter chosen,
  /// empty space clicked.
  void clearEffectSelection() {
    selectedEffectsLayer = null;
    if (selectedEffects.value.isEmpty) return;
    selectedEffects.value = const [];
    notifyListeners();
  }

  /// Add [layer] to the selection, or take it out again — Shift-click.
  void toggleSelected(LayerReference layer) {
    final id = layer.internallayerId;
    final next = [
      for (final held in selectedLayers.value)
        if (held.internallayerId != id) held,
    ];
    if (next.length == selectedLayers.value.length) next.add(layer);
    setSelection(next);
  }

  void clearSelection() => setSelection(const []);

  /// The turn a Rotation-tool drag is part way through, by layer id (K-230).
  ///
  /// The picture is previewed at the new angle while the drag is in flight, but
  /// the document still holds the old one — so the wireframe drawn from the
  /// document lagged the picture and only caught up on release. The tool that
  /// is turning publishes here and the gizmo that draws the boxes reads it; the
  /// two are different widgets in different layers of the Viewer's stack, and
  /// this is the one value they share. Empty whenever nothing is turning.
  final ValueNotifier<Map<UuidValue, double>> liveRotations =
      ValueNotifier(const {});

  /// The line a Type edit is part way through, by layer id (K-232).
  ///
  /// Published for the same reason as [liveRotations]: what is being typed is
  /// previewed on the picture while the document still holds the old document,
  /// so a box measured from the document does not grow as the words do. Empty
  /// whenever nothing is being typed.
  final ValueNotifier<Map<UuidValue, ({String text, double size})>> liveText =
      ValueNotifier(const {});

  /// The transform a value scrub is part way through, by layer id.
  ///
  /// The third of the same family, and for the reason [liveRotations] gives:
  /// dragging Position or Scale — in the property rows or on a curve in the
  /// graph — previews the *picture* at the new value while the document still
  /// holds the old one, so the box drawn from the document sat still until the
  /// drag was released. The row that is dragging publishes the provisional
  /// transform it already built for the preview, and the boxes read it.
  ///
  /// At most one layer at a time: a gesture is one property of one layer
  /// (see `previewChannelEdits`). Empty whenever nothing is being scrubbed —
  /// and it must be emptied on release, or the box would hold the last
  /// provisional value for ever.
  final ValueNotifier<Map<UuidValue, BridgeTransform>> liveTransforms =
      ValueNotifier(const {});

  /// Forget layers that are no longer in the composition (K-238).
  ///
  /// **Why this is not merely tidy.** A selection is not only a highlight — it
  /// is the answer to "which layer does this tool act on?". Undo a shape layer
  /// and the layer went, but its id stayed selected, so the next shape drag
  /// still believed a layer was selected and tried to draw a *mask* on one that
  /// no longer existed. The engine refused, the refusal was swallowed, and the
  /// drag did nothing: the tool had simply stopped working, with nothing on
  /// screen to say why.
  ///
  /// Undo is only the easiest way to see it. Deleting a layer from the
  /// Timeline, closing a comp, or any edit that removes a layer leaves the same
  /// stale name behind, which is why this is answered once, here, from the
  /// model — rather than at each of the places a layer can vanish.
  ///
  /// An empty model means nothing is loaded yet rather than everything has
  /// gone, so the selection is left alone: clearing it there would drop the
  /// selection on every rebind.
  void _dropVanishedFromSelection() {
    final held = selectedLayers.value;
    if (held.isEmpty) return;
    final live = model.heldLayers;
    if (live.isEmpty) return;
    final alive = {for (final entry in live) entry.layer.internallayerId};
    final kept = [
      for (final layer in held)
        if (alive.contains(layer.internallayerId)) layer,
    ];
    if (kept.length == held.length) return;
    setSelection(kept);
  }

  /// Keep the list honest when something sets the primary on its own.
  void _syncSelection() {
    final primary = selectedLayer.value;
    // Whichever way round the two notifiers were set, the selection has just
    // changed, and it is part of the session (see [rememberSession]).
    rememberSession();
    if (primary == null) {
      if (selectedLayers.value.isNotEmpty) selectedLayers.value = const [];
      return;
    }
    if (selectedLayerIds.contains(primary.internallayerId)) return;
    selectedLayers.value = List.unmodifiable([primary]);
  }

  /// The frame every panel renders and previews at.
  ///
  /// Held here rather than inside the Timeline because it is not the Timeline's
  /// alone: the Effect controls panel previews a drag at the playhead, and its
  /// preview landing on a different frame from the one on screen would show the
  /// wrong picture. A notifier so a scrub redraws only what watches it.
  ValueNotifier<int> playheadFrame = ValueNotifier(0);

  /// What fraction of comp resolution the Viewer is actually showing, which is
  /// the `scale` every render request carries. 1.0 until the Viewer has been laid
  /// out and can measure itself.
  ///
  /// This is why a Viewer in a small panel is cheap: the engine decodes and
  /// composites at the size being displayed rather than always at comp
  /// resolution. It is the frb counterpart of v0's `effectivePreviewScale`, minus
  /// the adaptive quality tier (K-171), which is not ported yet — so this tracks
  /// the panel size only, not measured render cost.
  ///
  /// A getter, not a field, because two separate things decide it: the panel
  /// measures itself ([reportViewerScale]) and the user chooses a preview
  /// resolution ([previewResolution]). Resolving them here means a change to
  /// either is in force on the very next render request, with nothing to keep
  /// in step.
  double get viewerScale => previewResolution.scaleFor(_panelScale);

  /// The scale the *panel* implies, last time the Viewer laid itself out.
  double _panelScale = 1.0;

  /// Called by the Viewer as it lays out. Clamped to (0, 1]: rendering *above*
  /// comp resolution would cost more for no visible gain, and a zero or negative
  /// scale is meaningless.
  ///
  /// **A panel that has changed size asks for the frame again** (K-430). On
  /// Auto the scale is whatever the panel could show at the moment it laid
  /// itself out, and the first layout of a session happens at whatever size the
  /// window opened at — so the first frame stayed at that scale until something
  /// else moved, because growing a panel is neither an edit nor a move of the
  /// playhead and nothing else asks. Compared at 1 % granularity, which is how
  /// the engine keys a frame's scale: a smaller difference would name a frame
  /// already in hand.
  ///
  /// A fixed tier is left alone: it is a raster reduction the user chose, and
  /// the panel's size has no say in it.
  ///
  /// [settled] is false while the Viewer's zoom is in flight. A flight is dozens
  /// of layouts, and a frame asked for at each of them is dozens of frames
  /// nobody ever sees.
  void reportViewerScale(double scale, {bool settled = true}) {
    if (!scale.isFinite || scale <= 0) return;
    final was = _panelScale;
    _panelScale = scale > 1.0 ? 1.0 : scale;
    if (!settled || previewResolution.fraction != null) return;
    if ((_panelScale * 100).round() == (was * 100).round()) return;
    // After this frame, not during it: this runs from the Viewer's layout, and
    // a render request made there would rebuild the tree it is measuring.
    WidgetsBinding.instance.addPostFrameCallback((_) => requestFrame());
  }

  /// The preview resolution of each composition, by id (K-357, docs/07 §2.2
  /// item 2). Per comp because it is a way of *working on* one — a heavy shot
  /// wants Quarter while the title card beside it does not — and it rides the
  /// session blob beside [viewerLooks] rather than the document, because
  /// choosing it is not an edit and must never reach an export (glossary §5).
  final Map<String, PreviewResolution> previewResolutions = {};

  /// How many pixels the engine is asked for, for the fronted comp.
  ///
  /// Auto until something says otherwise: it renders what the panel can show,
  /// which is what the Viewer has always in fact done.
  PreviewResolution get previewResolution =>
      previewResolutions[_selectedComp?.internalid.toString()] ??
      PreviewResolution.auto;

  /// Choose the preview resolution for the fronted comp, and ask for the frame
  /// again — the setting changes what the *next* frame is made of, so without
  /// the ask the picture would not change until something else moved.
  void setPreviewResolution(PreviewResolution resolution) {
    final id = _selectedComp?.internalid.toString();
    if (id == null || previewResolution == resolution) return;
    if (resolution == PreviewResolution.auto) {
      previewResolutions.remove(id);
    } else {
      previewResolutions[id] = resolution;
    }
    // The View menu ticks the one in force, so the bar has to be rebuilt.
    notifyListeners();
    rememberSession();
    requestFrame();
  }

  /// A named magnification the Viewer has been asked to take (docs/07 §2.2).
  ///
  /// A notifier for the same reason as [togglePlayRequest]: the magnification
  /// belongs to the Viewer panel — "fit" cannot be worked out without the
  /// panel's size — and the shell must not have to reach into a panel that may
  /// not be mounted. The serial makes two identical requests in a row two
  /// events rather than one, so pressing Zoom in twice zooms twice.
  final ValueNotifier<(int, ViewerZoomCommand)?> viewerZoomRequest =
      ValueNotifier(null);

  int _viewerZoomRequests = 0;

  void requestViewerZoom(ViewerZoomCommand command) =>
      viewerZoomRequest.value = (++_viewerZoomRequests, command);

  /// The armed dropper, or null when the tool is not armed (docs/07 §7).
  ///
  /// One at a time, and held at the session level rather than inside the Viewer:
  /// the tool is armed from a parameter row in another panel entirely, and the
  /// Viewer must not have to be mounted for that click to be harmless.
  final ValueNotifier<DropperArm?> dropper = ValueNotifier(null);

  /// The window of pixels the last [requestDropperSample] read back, or null
  /// before the first reply. Cleared when the tool disarms, so a fresh arm
  /// never opens on the previous pick's pixels.
  ///
  /// A window, not a pixel: the magnifier cuts its own nine-by-nine out of this
  /// as the pointer moves, and only asks again when the pointer nears its edge
  /// (see `windowCovers`). That is what keeps a sweep across the picture to a
  /// handful of reads instead of one per mouse move.
  final ValueNotifier<BridgeSampledPixels?> dropperPatch = ValueNotifier(null);

  /// Arm the dropper. Replaces whatever was armed — two pending picks would
  /// leave the next click ambiguous.
  void armDropper(DropperArm arm) {
    dropperPatch.value = null;
    dropper.value = arm;
  }

  /// Put the dropper away, picked or not.
  void disarmDropper() {
    dropper.value = null;
    dropperPatch.value = null;
  }

  /// Ask the engine for a window of pixels around the point `(u, v)` of the
  /// picture, each a fraction from 0 to 1. The answer arrives on the worker
  /// stream and lands in [dropperPatch]; nothing here waits for it.
  ///
  /// A fraction rather than a pixel because the frontend cannot know which
  /// raster will be read — a reduced-resolution preview has its own grid, and
  /// the reply is what says which one it used.
  ///
  /// Called only when the window in hand cannot answer — the caller checks
  /// first — so this is a handful of calls per pick, not one per mouse move.
  void requestDropperSample(double u, double v) {
    final comp = selectedComp;
    final arm = dropper.value;
    if (comp == null || arm == null) return;
    try {
      comp.samplePixels(
        frame: BigInt.from(playheadFrame.value),
        u: u,
        v: v,
        window: dropperWindow,
        scale: viewerScale,
        layer: arm.sampleLayer,
      );
    } catch (_) {
      // No worker, or a composition that has gone away. The next pointer move
      // asks again; there is nothing to recover here.
    }
  }

  StreamSubscription? sub;
  StreamSubscription? _changes;

  /// The session's engine-facing state. Held because the comp list it caches
  /// is what says which comps still exist (K-184).
  final LumitState _app;

  LumitUiState(LumitState state, {Workspace? workspace})
      : _app = state,
        workspace = workspace ?? (Workspace()..load()) {
    // The language, before anything is built: `t` is a plain global
    // (l10n/strings.dart), so it has to hold the right strings by the time the
    // first widget asks for one. Registered ahead of `notifyListeners` below so
    // that a language change has already landed when the rebuild it triggers
    // runs — listeners fire in the order they were added.
    _applyLanguage();
    this.workspace.addListener(_applyLanguage);
    // Appearance and layout live in the workspace, so a change there is a
    // change here as far as any listening widget is concerned.
    this.workspace.addListener(notifyListeners);
    // Floating windows read and write where they were left through this
    // (K-242); the controls file has no other way to reach the store.
    modalPlacementStore = this.workspace;
    selectedLayer.addListener(_syncSelection);
    // A layer that has gone must leave the selection with it (K-238). The
    // model is the one place that knows which layers exist, so the pruning
    // hangs off its refresh rather than off each of the several ways a layer
    // can disappear.
    model.addListener(_dropVanishedFromSelection);
    // And the same for the comp the model itself is bound to: it can be undone
    // out of existence while it is the one being looked at.
    model.addListener(_frontLiveCompIfFrontedOneHasGone);
    // A project being adopted — opened, or made new — is where the saved
    // session is put back. The engine state is the only thing that knows a
    // document has been swapped underneath us. The document loaded *now* is
    // the one this shell starts on, so it does not count as a swap: without
    // this the first edit of the session would read as a new project and
    // clear the fronted comp and the selection.
    _sessionProject = _app.project;
    _app.addListener(_adoptProjectSession);
    // Where the playhead was left is worth keeping, and it moves far too often
    // to write down each time. So it is captured when the user steps away from
    // the window and when they close it, alongside the deliberate acts below.
    _lifecycle = AppLifecycleListener(
      onInactive: rememberSession,
      onExitRequested: () async {
        rememberSession();
        return AppExitResponse.exit;
      },
    );
    // The keymap: restored from the workspace if the user has changed one,
    // otherwise the engine's shipped defaults (K-199). Held here because
    // every keypress goes through it and the settings page edits it, so it
    // wants the same lifetime as the rest of the session's UI state.
    keymap = KeymapState(workspace: this.workspace);
    // The cache budgets: live engine state with no store behind it, so the
    // settings file carries the user's choice and hands it back here (K-194's
    // sizing only picks the *default*). Null means untouched — leave the
    // engine on its own default rather than writing today's default into the
    // file forever.
    final perf = this.workspace.performance;
    final ramBudget = perf.cacheBudgetBytes;
    if (ramBudget != null) setCacheBudget(bytes: BigInt.from(ramBudget));
    final vramBudget = perf.vramBudgetBytes;
    if (vramBudget != null) setVramCacheBudget(bytes: BigInt.from(vramBudget));
    final diskBudget = perf.diskBudgetBytes;
    if (diskBudget != null) setDiskCacheBudget(bytes: BigInt.from(diskBudget));
    // Where the parked frames go. Restored the same way, and by name rather
    // than by index so a reordered enum cannot silently move a user's cache.
    final where = perf.diskCacheLocation;
    if (where != null) {
      setDiskCacheLocation(
        location: cacheLocationFromName(where),
        folder: perf.diskCacheFolder ?? '',
      );
    }
    // The read model re-reads on every committed change — one bridge call —
    // and every panel that draws layers repaints from it (K-184).
    _changes = state.onChange.listen((_) {
      clearCompTimeCache();
      model.refresh();
    });
    sub = state.onWorkerResponse.listen((msg) {
      // Any reply at all means the new project's worker is up and answering,
      // which is what the opening card is waiting on. Not the frame alone: a
      // first render that faults would otherwise leave the shell covered.
      state.previewReady();
      switch (msg) {
        case WorkerResponse_RenderedDMABuf frame:
          previewTier.value = frame.field0.tier;
          _showDmabuf(frame.field0);
        case WorkerResponse_RenderedSharedTexture frame:
          previewTier.value = frame.field0.tier;
          _showSharedTexture(frame.field0);
        // Scope traces ride the same stream; the Scopes panel subscribes to it
        // directly, so there is nothing for the Viewer to do with one.
        case WorkerResponse_Scope():
          break;
        // Playback ran off the end on its own. Stopping because the *user* asked
        // needs no message — `stopPlayback` already set the flag.
        case WorkerResponse_PlaybackEnded():
          playing.value = false;
          // Running off the end returns the playhead too (K-254): where you are
          // when the transport stops should not depend on whether you stopped
          // it or the composition ran out.
          _returnPlayhead();
        case WorkerResponse_CacheFilled():
          cacheChanged.value++;
        // The pixels under the dropper. Held rather than acted on: the
        // magnifier draws whatever the last read said, and the click that
        // picks reads it from here.
        case WorkerResponse_Sampled(:final field0):
          dropperPatch.value = field0;
        // How far the frame being waited on has got. The engine sends these
        // only for a frame somebody is waiting on — never during playback —
        // and the tracker decides whether it is slow enough to draw.
        case WorkerResponse_RenderProgress(:final field0):
          previewProgress.report(field0);
        // What the frame just made cost. Only sent while something is showing
        // the numbers (`RenderTimings.setMeasuring`).
        case WorkerResponse_FrameProfile(:final field0):
          renderTimings.report(field0);
      }
    });
  }

  /// Linux zero-copy: register the DMA-BUF and show its texture. The first
  /// positional argument is the controller's identity key, and on this path the
  /// fd serves as that key — a non-null `fd` is also what tells the controller to
  /// send the DMA-BUF argument set rather than the DXGI one.
  void _showDmabuf(BridgeSharedFrameInfoLinux f) {
    controller
        .ensureRegistered(f.fd, f.width, f.height,
            fd: f.fd,
            stride: f.stride,
            offset: f.offset,
            fourcc: f.drmFourcc,
            modifier: f.modifier.toInt())
        .then((id) => _adoptTexture(id, f.frame.toInt()));
  }

  /// Windows and macOS zero-copy: register the surface by the one integer that
  /// names it — an NT handle for the shared D3D12 texture there, an `IOSurfaceID`
  /// here (K-195). One case for both, because the payload is the same shape.
  /// Leaving `fd` null is what selects the handle argument set.
  void _showSharedTexture(BridgeSharedFrameInfo f) {
    controller
        .ensureRegistered(f.handle.toInt(), f.width, f.height)
        .then((id) => _adoptTexture(id, f.frame.toInt()));
  }

  /// A registered texture is now current: mark a frame available and, if the id
  /// changed, point the Viewer at it.
  void _adoptTexture(int? id, int frame) {
    _arrived(frame);
    if (id == null) return;
    controller.frameReady();
    if (viewerFrameid.value != id) viewerFrameid.value = id;
  }

  @override
  void dispose() {
    _app.removeListener(_adoptProjectSession);
    _lifecycle.dispose();
    sub?.cancel();
    _changes?.cancel();
    tools.dispose();
    layerBounds.dispose();
    // The progress tracker owns a timer — the delay that decides whether a
    // slow frame is slow enough to draw a bar for. Cancelling the subscription
    // above stops new reports, but a report that arrived a moment earlier has
    // already started one, and an uncancelled timer outlives the thing that
    // set it. In the application that is a small leak per project session; in
    // the frb tests it is a failure, and one that lands on whichever test
    // happens to be running when it fires rather than the one that caused it.
    previewProgress.dispose();
    model.dispose();
    cacheChanged.dispose();
    solveLanded.dispose();
    previewTier.dispose();
    viewerFrameid.dispose();
    selectedLayer.removeListener(_syncSelection);
    selectedLayer.dispose();
    selectedLayers.dispose();
    graphNode.dispose();
    activePanel.dispose();
    paletteRequest.dispose();
    consoleRequest.dispose();
    viewerZoomRequest.dispose();
    panelSearchRequest.dispose();
    super.dispose();
  }

  /// The comps open as Timeline tabs (docs/07 §4: one tab per open comp), in
  /// the order first fronted. Fronting a comp opens its tab; closing a tab
  /// only closes the tab — the comp stays in the project.
  final List<UuidValue> openComps = [];

  /// The comp fronted before this one, so a comp that vanishes under the user
  /// can put them back where they came from rather than somewhere arbitrary.
  UuidValue? _previousComp;

  void setSelectedComp(CompositionReference? reference) {
    if (reference != null && !openComps.contains(reference.internalid)) {
      openComps.add(reference.internalid);
    }
    if (reference?.internalid != _selectedComp?.internalid) {
      _previousComp = _selectedComp?.internalid;
    }
    _selectedComp = reference;
    model.bind(reference);
    // Each comp is looked at its own way (K-314), so fronting one is what puts
    // its exposure and tone map back on the engine's renderer — which holds
    // exactly one view, for whatever the Viewer is showing.
    pushViewerLook();
    rememberSession();
    notifyListeners();
  }

  // --- How the Viewer is looking (K-314) -----------------------------------

  /// Exposure and tone map per composition id. Not in the document: a way of
  /// looking is not an edit, so this rides in the session blob (see
  /// [session]) and never in an op.
  final Map<String, ViewerLook> viewerLooks = {};

  /// How the fronted comp is being looked at, neutral until something says
  /// otherwise.
  ///
  /// The one place the stored look becomes the look in use, which is why the
  /// tone map setting is honoured here and nowhere else: the Viewer bar, the
  /// engine push and the button all read this, so they cannot disagree. With
  /// the setting off the tone map is false whatever the comp stored — a
  /// session saved while it was engaged would otherwise be stranded with no
  /// button to turn it off. Only the reading is gated, not the store, so
  /// turning the setting back on finds the comp as it was — until the exposure
  /// is moved while the button is away, which writes the pair back as seen.
  ViewerLook get viewerLook {
    final stored =
        viewerLooks[_selectedComp?.internalid.toString()] ?? neutralLook;
    if (workspace.interface.showToneMap) return stored;
    return (stops: stored.stops, toneMap: false);
  }

  /// The whole look the engine was last told — exposure, tone map and the
  /// transparency grid — or null when a freshly adopted worker has been told
  /// nothing yet. One record for the three, because they travel as one
  /// message ([pushViewerLook]) and a push can be skipped only when *all* of
  /// it is already in force.
  ({double stops, bool toneMap, bool grid, String roi})? _pushedView;

  /// The Viewer's **region of interest** per comp (K-362): the sub-rectangle
  /// the engine composites, as comp fractions `[u0, v0, u1, v1]`. Rides the
  /// session beside [previewResolutions] rather than the document, for the
  /// same reason — choosing where to look is not an edit, and must never reach
  /// an export.
  final Map<String, List<double>> regionsOfInterest = {};

  /// Whether the next drag on the picture sweeps out a region of interest
  /// (K-362). View state rather than panel state only because two widgets need
  /// it — the bar that arms it and the layer that takes the drag — and
  /// threading a transient flag between them through the stage is more
  /// machinery than the flag is worth. Never saved: arming is a thing you are
  /// in the middle of.
  bool _armingRegion = false;
  bool get armingRegion => _armingRegion;
  set armingRegion(bool on) {
    if (_armingRegion == on) return;
    _armingRegion = on;
    notifyListeners();
  }

  /// The fronted comp's region, or null for the whole frame.
  List<double>? get regionOfInterest {
    final id = _selectedComp?.internalid.toString();
    return id == null ? null : regionsOfInterest[id];
  }

  /// Set (or with null, clear) the fronted comp's region and re-render. A
  /// region that is not four numbers, is inside-out, or covers everything is
  /// no region — the engine says the same, and agreeing here keeps the button's
  /// lit state honest.
  void setRegionOfInterest(List<double>? region) {
    final id = _selectedComp?.internalid.toString();
    if (id == null) return;
    final ok = region != null &&
        region.length == 4 &&
        region.every((v) => v.isFinite) &&
        region[2] - region[0] > 0.001 &&
        region[3] - region[1] > 0.001 &&
        (region[0] > 0 || region[1] > 0 || region[2] < 1 || region[3] < 1);
    if (ok) {
      regionsOfInterest[id] = [
        region[0].clamp(0.0, 1.0),
        region[1].clamp(0.0, 1.0),
        region[2].clamp(0.0, 1.0),
        region[3].clamp(0.0, 1.0),
      ];
    } else {
      regionsOfInterest.remove(id);
    }
    _armingRegion = false;
    notifyListeners();
    rememberSession();
    pushViewerLook();
  }

  /// The Viewer's **overlays** per comp (K-416, docs/07 §2.2 items 5–6): the
  /// proportional grid and the title/action safe rectangles, drawn over the
  /// picture by the display and by nothing else.
  ///
  /// Keyed by comp exactly as [regionsOfInterest] is, and for the same reason —
  /// which marks you want over a shot belong to that shot. Nothing here crosses
  /// the bridge: the engine's picture is untouched, so unlike the region there
  /// is no push and no re-render, only a repaint. Session only for now; keeping
  /// a comp's overlays with the project is owed (docs/TODO.md).
  final Map<String, ({bool grid, bool safeAreas})> viewerOverlaysByComp = {};

  /// The fronted comp's overlays — nothing drawn, until something is asked for.
  ({bool grid, bool safeAreas}) get viewerOverlays =>
      viewerOverlaysByComp[_selectedComp?.internalid.toString()] ??
      (grid: false, safeAreas: false);

  /// Turn one overlay on or off, leaving the other as it is. Two named
  /// arguments rather than a whole record, for [setViewerStops]'s reason: a
  /// caller that rebuilt the pair from what it was *drawn* with would carry a
  /// stale reading for the other half into the write.
  void setViewerOverlays({bool? grid, bool? safeAreas}) {
    final id = _selectedComp?.internalid.toString();
    if (id == null) return;
    final now = viewerOverlays;
    final next =
        (grid: grid ?? now.grid, safeAreas: safeAreas ?? now.safeAreas);
    if (next.grid || next.safeAreas) {
      viewerOverlaysByComp[id] = next;
    } else {
      viewerOverlaysByComp.remove(id);
    }
    notifyListeners();
  }

  /// Whether the Viewer's transparency grid is up (K-352). While it is, the
  /// engine leaves the comp's background colour out of the composite, so
  /// pixels nothing covers arrive transparent and the grid shows through.
  ///
  /// Held here rather than in the Viewer panel because the engine has to be
  /// told: it rides [pushViewerLook]'s one look message, and project adoption
  /// clears [_pushedView] so a worker just born — which starts opaque — is
  /// told afresh on the first front.
  bool viewerGrid = true;

  /// Flip the transparency grid. The push and the re-render are
  /// [pushViewerLook]'s: the grid is part of the one look message, and the
  /// change of record is what makes it ask for the frame again.
  void setViewerGrid(bool on) {
    if (viewerGrid == on) return;
    viewerGrid = on;
    notifyListeners();
    pushViewerLook();
  }

  /// Set the exposure, leaving the tone map as it is; and the mirror of it.
  ///
  /// Two setters rather than one taking a whole [ViewerLook], because each
  /// control must change only its own half. A control that rebuilt the pair
  /// from the value it was *drawn* with would carry a stale reading for the
  /// other half into the write — two changes between two rebuilds, and the
  /// second undoes the first.
  void setViewerStops(double stops) =>
      setViewerLook((stops: stops, toneMap: viewerLook.toneMap));

  void toggleViewerToneMap() =>
      setViewerLook((stops: viewerLook.stops, toneMap: !viewerLook.toneMap));

  /// Set how the fronted comp is looked at, tell the engine, and write it down.
  void setViewerLook(ViewerLook look) {
    final id = _selectedComp?.internalid.toString();
    if (id == null) return;
    if (look == neutralLook) {
      viewerLooks.remove(id);
    } else {
      viewerLooks[id] = look;
    }
    pushViewerLook();
    rememberSession();
    notifyListeners();
  }

  /// Tell the engine what the Viewer is looking through — exposure, tone map
  /// and the transparency grid, one message carrying the whole look — and ask
  /// for the frame again, because a setting changes what the *next* frame is
  /// made of.
  ///
  /// A look the renderer already holds ([_pushedView]) is nothing to say and
  /// nothing to re-render, so it is skipped — which is every fronting where
  /// nothing changed, and what keeps an exposure drag to one message per
  /// actual movement. The record clears on project adoption, because a new
  /// worker is born neutral-and-opaque and must be told afresh.
  void pushViewerLook() {
    final comp = selectedComp;
    if (comp == null) return;
    final look = viewerLook;
    final roi = regionOfInterest;
    final target = (
      stops: look.stops,
      toneMap: look.toneMap,
      grid: viewerGrid,
      // Compared as text: a record holding a list compares by identity, so two
      // equal regions would look different and re-push on every fronting.
      roi: roi?.join(',') ?? '',
    );
    if (target == _pushedView) return;
    try {
      comp.setViewerLook(
        stops: target.stops,
        toneMap: target.toneMap,
        transparentBackground: target.grid,
        region: roi == null ? null : Float32List.fromList(roi),
      );
    } catch (_) {
      // No worker yet, or a comp that has gone. The next change asks again —
      // and _pushedView stays as it was, so the ask is not skipped.
      return;
    }
    _pushedView = target;
    requestFrame();
  }

  // --- The per-project session ---------------------------------------------
  //
  // Where the user had got to in *this* document: the comps on the tab strip,
  // which one was fronted, where the playhead sat, what was selected. It is
  // kept in the workspace store keyed by the project's path rather than in the
  // `.lum`, because none of it is the document — a project file must stay
  // byte-identical between two saves of the same work and must not carry one
  // machine's habits to another (docs/10 §1.1, §2). The panel arrangement and
  // which panel each tab group fronts are already persisted there too, app-wide.

  /// The project whose session is on screen. Compared by identity, so a
  /// document swapped underneath the shell is noticed the moment it is adopted.
  ProjectReference? _sessionProject;

  late final AppLifecycleListener _lifecycle;

  /// Write down where the user is. A project with no file has nowhere to be
  /// written to — the sessions are keyed by path — so this is a no-op until it
  /// has been saved once.
  void rememberSession() {
    // Restoring moves the fronted comp and the selection, and each of those
    // moves would be written back — over the very session being read.
    if (_restoring) return;
    final path = _app.project?.path();
    if (path == null) return;
    workspace.rememberSession(path, session());
  }

  /// Where the user is, as the thing that gets written down: the tab strip, the
  /// fronted comp, the playhead, the selection, and how the panels are arranged.
  SavedSession session() => SavedSession(
        openComps: [for (final id in openComps) id.toString()],
        activeComp: _selectedComp?.internalid.toString(),
        frame: playheadFrame.value,
        selectedLayer: selectedLayer.value?.internallayerId.toString(),
        dock: workspace.dock.toJson(),
        viewerLooks: Map.of(viewerLooks),
        previewResolutions: {
          for (final e in previewResolutions.entries) e.key: e.value.name,
        },
        regionsOfInterest: {
          for (final e in regionsOfInterest.entries) e.key: List.of(e.value),
        },
      );

  /// The same thing as JSON, for the copy that goes inside the `.lum` so it
  /// travels with a project shared with someone else (K-245).
  String sessionJson() => jsonEncode(session().toJson());

  /// Put the saved session back after a project is opened, and start from
  /// nothing when a new one is made.
  ///
  /// Every id is checked against the document that actually loaded before it is
  /// used: a comp or layer deleted since the session was written must leave the
  /// user on a sensible default, never on a reference the engine has never
  /// heard of.
  bool _restoring = false;

  /// The arrangement the project file itself carries, or null when it has none
  /// or none that can be read.
  SavedSession? _embeddedSession(ProjectReference project) {
    try {
      final json = project.uiState();
      if (json == null) return null;
      final decoded = jsonDecode(json);
      if (decoded is! Map) return null;
      return SavedSession.fromJson(decoded.cast<String, dynamic>());
    } catch (_) {
      // A project written by another build may describe an interface this one
      // does not have. Opening it must still work; it simply opens arranged the
      // way this machine already was.
      return null;
    }
  }

  /// Put the panels where the session says, if it says anything about them.
  ///
  /// A layout naming a panel this build has never heard of is dropped whole
  /// rather than half-applied — the arrangement is a hint, and the one on
  /// screen is a perfectly good fallback.
  void _applyDock(Map<String, dynamic>? json) {
    if (json == null) return;
    try {
      final parsed = DockNode.fromJson(json);
      if (parsed is! DockSplit) return;
      workspace.dock = parsed;
      workspace.touch();
    } catch (_) {
      // Left as it was.
    }
  }

  void _adoptProjectSession() {
    final project = _app.project;
    if (identical(project, _sessionProject)) return;
    _sessionProject = project;

    _restoring = true;
    try {
      // Nothing from the previous document may outlive it: its comp ids, its
      // playhead and its selection all belong to a project no longer loaded.
      openComps.clear();
      clearSelection();
      playheadFrame.value = 0;
      viewerLooks.clear();
      previewResolutions.clear();
      // A new project is a new worker, and a new worker is born knowing
      // nothing of this session's look — the null record is what makes the
      // first front tell it everything, the grid included.
      _pushedView = null;
      setSelectedComp(null);

      final path = project?.path();
      if (path == null) return;
      workspace.rememberProject(path);

      // This machine's own record of the project comes first: it is the more
      // recent account of what *this* user was doing, and it is kept up to date
      // between saves. The one in the file is what a project arriving from
      // somebody else brings with it (K-245), so it answers exactly when there
      // is no local record — the first time this project is opened here.
      final session = workspace.sessionFor(path) ?? _embeddedSession(project!);
      if (session == null) return;
      _applyDock(session.dock);

      final known = {
        for (final (comp, _) in _app.comps()) comp.internalid.toString(): comp,
      };
      // Only for comps this document still has: a look kept against a comp id
      // that has gone would be written back out for ever.
      viewerLooks.addEntries(
        session.viewerLooks.entries.where((e) => known.containsKey(e.key)),
      );
      // Same rule for the per-comp resolutions, plus: a name this build does
      // not have (a project written by a newer one) simply reads as Auto
      // rather than stopping the project from opening.
      for (final e in session.previewResolutions.entries) {
        if (!known.containsKey(e.key)) continue;
        for (final r in PreviewResolution.values) {
          if (r.name == e.value && r != PreviewResolution.auto) {
            previewResolutions[e.key] = r;
          }
        }
      }
      // The regions, checked against the comps that actually loaded — the
      // same rule every other id in a session gets (K-362).
      for (final e in session.regionsOfInterest.entries) {
        if (known.containsKey(e.key)) {
          regionsOfInterest[e.key] = List.of(e.value);
        }
      }
      for (final id in session.openComps) {
        final comp = known[id];
        if (comp != null) openComps.add(comp.internalid);
      }
      final front = known[session.activeComp] ??
          (openComps.isEmpty ? null : known[openComps.first.toString()]);
      setSelectedComp(front);
      playheadFrame.value = session.frame < 0 ? 0 : session.frame;

      final wanted = session.selectedLayer;
      if (front == null || wanted == null) return;
      for (final layer in front.getLayers()) {
        if (layer.internallayerId.toString() == wanted) {
          setSelection([layer]);
          break;
        }
      }
    } finally {
      _restoring = false;
      // A project that opens with no composition fronted has no picture to
      // wait for, so the opening card would stand for ever waiting on a frame
      // nobody is going to ask for. In the `finally` because every early
      // return above is one of those cases.
      if (selectedComp == null) _app.previewReady();
    }
  }

  /// **A comp can be taken out from under the user.** Pre-compose, step into
  /// the new comp, undo: the layer comes back and the comp it pointed at stops
  /// existing, with the Timeline still fronting it. Every panel then reads a
  /// comp the engine has never heard of, which is what put a bridge error on
  /// screen where the timeline should be.
  ///
  /// So the same rule the layer selection follows (K-238) applies to the
  /// fronted comp: what has gone cannot stay fronted. Where to go instead, in
  /// order — the comp the user was in before this one, if it is still there;
  /// else the nearest open tab, left first and then right; else nothing
  /// fronted at all, which is the state the shell starts in and draws fine.
  void _frontLiveCompIfFrontedOneHasGone() {
    if (!model.compGone) return;
    final gone = _selectedComp!.internalid;
    final known = {
      for (final (comp, _) in _app.comps()) comp.internalid: comp,
    };
    final where = openComps.indexOf(gone);
    final at = where < 0 ? 0 : where;
    openComps.remove(gone);

    // Where the user came from, then leftwards from where the gone tab stood,
    // then rightwards — after the removal the tab that stood at `at` is the
    // right-hand neighbour.
    final order = <UuidValue?>[
      _previousComp,
      for (var i = at - 1; i >= 0; i--) openComps[i],
      for (var i = at; i < openComps.length; i++) openComps[i],
    ];
    CompositionReference? next;
    for (final id in order) {
      final candidate = id == null ? null : known[id];
      // Asked of the engine, not of the cached item walk: the walk is only
      // re-read when the change stream says the tree moved, and this runs on
      // the model's refresh, which can be the earlier of the two. Fronting a
      // comp that is *also* gone would land straight back here.
      if (candidate != null && _stillThere(candidate)) {
        next = candidate;
        break;
      }
    }
    _previousComp = null;
    setSelectedComp(next);
  }

  bool _stillThere(CompositionReference comp) {
    try {
      comp.getSettings();
      return true;
    } catch (_) {
      return false;
    }
  }

  /// Drop a comp's tab where [target]'s tab sits. The strip is drawn in this
  /// list's order, so moving the entry is the whole reorder — and it rides
  /// along in the session, like the rest of the tab strip.
  void moveComp(UuidValue id, UuidValue target) {
    final from = openComps.indexOf(id);
    final to = openComps.indexOf(target);
    if (from < 0 || to < 0 || from == to) return;
    openComps.removeAt(from);
    openComps.insert(to, id);
    rememberSession();
    notifyListeners();
  }

  /// Close a comp's Timeline tab. When the closed tab was fronted, [fallback]
  /// — the tab bar's nearest remaining neighbour — fronts instead.
  void closeComp(UuidValue id, {CompositionReference? fallback}) {
    openComps.remove(id);
    if (_selectedComp?.internalid == id) {
      setSelectedComp(fallback);
    } else {
      rememberSession();
      notifyListeners();
    }
  }
}

class LumitAppNew extends StatelessWidget {
  final LumitState state;
  final LumitUiState uiState;

  /// Whether to open on the welcome screen (K-464). False when a project came
  /// in on the command line: that is already the answer the screen asks for.
  final bool welcome;

  const LumitAppNew(this.state, this.uiState, {super.key, this.welcome = true});

  @override
  Widget build(BuildContext context) {
    // WidgetsApp-level infrastructure only — no Material chrome
    // (docs/archive/flutter-port/04 "Why not Material chrome"). `ThemeData.dark()`
    // was doing real damage here: Material's own greys showed through wherever a
    // panel did not paint, so the shell read as a Material app with Lumit panels
    // in it rather than as Lumit. The backdrop is `surface0` from the theme.
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      // Lumit's own strings come from the `l10n` global rather than from
      // context (l10n/strings.dart); these delegates are still needed for the
      // parts of Flutter that do ask the tree — text selection menus, the
      // Material and Cupertino widgets under a dialogue, and the text direction
      // a right-to-left language would want.
      locale: uiState.locale,
      localizationsDelegates: Strings.localizationsDelegates,
      supportedLocales: Strings.supportedLocales,
      home: ChangeNotifierProvider.value(
        value: state,
        child: ChangeNotifierProvider.value(
          value: uiState,
          // Rebuilt when the workspace changes, so the scale slider and the
          // scheme picker take effect as they are moved.
          child: ListenableBuilder(
            listenable: uiState,
            builder: (context, _) => ThemeScope(
              theme: uiState.theme,
              animationLevel: uiState.workspace.animationLevel,
              showTooltips: uiState.workspace.interface.showTooltips,
              child: Directionality(
                textDirection: TextDirection.ltr,
                child: ColoredBox(
                  color: uiState.theme.surface0,
                  // Settings → Interface → UI scale, the Flutter counterpart of
                  // egui's `set_pixels_per_point`: layout and hit-testing scale
                  // together (see widgets/ui_scale.dart).
                  child: UiScaleView(
                    scale: uiState.workspace.interface.uiScale,
                    child: Overlay(initialEntries: [
                      OverlayEntry(
                          builder: (context) => BootGate(welcome: welcome))
                    ]),
                  ),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// The boot splash, the welcome screen, and the shell behind them once both are
/// done (K-008, K-464).
///
/// **Each of the three is the window in turn**, never a card floating over a
/// half-built application: the shell is not put in the tree until boot has
/// finished and the user has said how they want to start, so nothing of the
/// application shows through, no dialogue — the first-run question above all —
/// can appear underneath, and no panel starts asking the engine for pictures
/// nobody can see yet.
///
/// The lines the splash streams are the engine's own boot log: the library
/// version, the ABI, and what this build was compiled with. That is the only
/// thing the engine can say about starting up — there is no notice stream to
/// subscribe to, only `boot_log` (docs/TODO.md) — so it is what the splash
/// shows, and a build with no bridge at all falls back to the canned list in
/// splash.dart.
class BootGate extends StatefulWidget {
  /// Whether to show the splash at all. False in the tests that drive the
  /// whole shell, which have no boot to wait for and would otherwise spend a
  /// second and a third of simulated time watching one.
  final bool splash;

  /// Whether to show the welcome screen after it. False when a `.lum` came in
  /// on the command line — somebody who double-clicked a project has already
  /// said what they want to open — and in the tests that drive the shell.
  final bool welcome;

  const BootGate({super.key, this.splash = true, this.welcome = true});

  @override
  State<BootGate> createState() => _BootGateState();
}

class _BootGateState extends State<BootGate> {
  late bool _booting = widget.splash;

  /// Whether the welcome screen is the window right now. Both answers have to
  /// agree: a `.lum` on the command line stands the screen down for this launch
  /// ([BootGate.welcome]), and Settings ▸ General stands it down for every
  /// launch (K-481). Read once, here, because the shell behind it is what the
  /// setting sends somebody to.
  late bool _welcoming;

  @override
  void initState() {
    super.initState();
    _welcoming = widget.welcome &&
        context.read<LumitUiState>().workspace.showWelcomeOnLaunch;
  }

  /// The engine's boot log, or empty where there is no engine to ask — a
  /// placeholder build, or a widget test with no library loaded. Read once:
  /// it is a bridge call, and it answers the same thing every time.
  late final List<String> _lines = _readBootLog();

  static List<String> _readBootLog() {
    try {
      return bootLog();
    } catch (_) {
      return const [];
    }
  }

  @override
  Widget build(BuildContext context) {
    if (_booting) {
      return SplashOverlay(
        lines: _lines,
        onDone: () {
          if (mounted) setState(() => _booting = false);
        },
      );
    }
    if (_welcoming) {
      return WelcomeScreenFrb(
        onDone: () {
          if (mounted) setState(() => _welcoming = false);
        },
      );
    }
    return const LumitAppView();
  }
}

class LumitAppView extends StatefulWidget {
  const LumitAppView({super.key});

  @override
  State<LumitAppView> createState() => _LumitAppViewState();
}

class _LumitAppViewState extends State<LumitAppView> {
  @override
  void initState() {
    super.initState();
    // Shortcuts are handled GLOBALLY, not through the focus tree. Every menu,
    // popup and palette lives in the Overlay outside this view's scope, so
    // any of them could walk focus away and never bring it back — and every
    // shortcut died until something was clicked (the space bar's recurring
    // funeral). A hardware-keyboard handler fires wherever focus is; the
    // focused-text-field guard inside _onKey keeps typing safe.
    HardwareKeyboard.instance.addHandler(_handleKey);
    // The pointer is tracked the same way — globally, not through the widget
    // tree. The Ctrl+Space console opens its ring at the mouse (K-325), and a
    // key event carries no position; a widget `Listener` missed everywhere no
    // widget claims the hit (the Viewer's texture, above all), so the console
    // kept opening at wherever the pointer had last crossed a panel. A global
    // route sees every pointer event regardless. One field write per event —
    // no setState, no bridge.
    GestureBinding.instance.pointerRouter.addGlobalRoute(_trackPointer);
    // A Lumit document copied while this window was away — in another Lumit
    // window, most of all — is picked up when the window comes back (K-302), so
    // Paste is live rather than greyed over something that is genuinely there.
    _clipboardWatch = AppLifecycleListener(
      onShow: () => context.read<LumitUiState>().adoptSystemClipboard(),
      onRestart: () => context.read<LumitUiState>().adoptSystemClipboard(),
    );
    // The first-run question (K-246), after the first frame so there is an
    // Overlay to put it in. It asks nothing on any later launch.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!mounted) return;
      final ui = context.read<LumitUiState>();
      // The update check follows the question rather than racing it: the
      // setup screen is where somebody may have just switched it off (K-296).
      maybeShowFirstRunFrb(context, ui.workspace)
          .then((_) => ui.maybeCheckForUpdates());
    });
  }

  AppLifecycleListener? _clipboardWatch;

  @override
  void dispose() {
    HardwareKeyboard.instance.removeHandler(_handleKey);
    GestureBinding.instance.pointerRouter.removeGlobalRoute(_trackPointer);
    _clipboardWatch?.dispose();
    super.dispose();
  }

  void _trackPointer(PointerEvent event) {
    if (event is PointerHoverEvent ||
        event is PointerMoveEvent ||
        event is PointerDownEvent) {
      lastKnownPointerPosition = event.position;
    }
  }

  bool _handleKey(KeyEvent event) {
    if (!mounted) return false;
    // The overlay swallows the pointer; keys are routed globally rather than
    // through the tree, so they have to be swallowed here. A command aimed at
    // the document being replaced has nothing left to run against, and one
    // aimed at a document being worked on would race the job doing it.
    final app = context.read<LumitState>();
    if (app.opening.value || app.busy.value != null) return true;
    return _onKey(
            context.read<LumitState>(), context.read<LumitUiState>(), event) ==
        KeyEventResult.handled;
  }

  @override
  Widget build(BuildContext context) {
    var uiState = context.watch<LumitUiState>();
    final state = context.watch<LumitState>();

    // The scope stays for the text fields: when one gives focus up, focus
    // falls back to the enclosing scope rather than to nothing.
    return FocusScope(
      autofocus: true,
      child: Stack(children: [
        _shell(uiState, state),
        // Over everything while a document is being read (see OpeningOverlay):
        // the shell behind it is still the previous project and swaps in one go.
        ValueListenableBuilder<bool>(
          valueListenable: state.opening,
          builder: (context, opening, _) =>
              opening ? const OpeningOverlay() : const SizedBox.shrink(),
        ),
        // The same card for a job working on the document that is already open
        // — beat detection. The two never overlap: nothing can be started
        // against a document that is still being read.
        BusyOverlay(busy: state.busy),
      ]),
    );
  }

  Widget _shell(LumitUiState uiState, LumitState state) => Column(
        children: [
          LumitMenuBarFrb(app: state),
          // The tools, under the menu and above everything else — where a
          // toolbar goes, and where docs/07 §1.7 puts it.
          const LumitToolBarFrb(),
          Expanded(
            child: DockWidget(
              root: uiState.split,
              buildPanel: (context, panel) => buildPanelBodyFrb(context, panel),
              // Persisted, so an arrangement survives a restart.
              onLayoutChanged: uiState.saveLayout,
              activePanel: uiState.activePanel,
            ),
          ),
          // The strip under the dock (docs/07 §1): the running export's
          // progress and Cancel, reachable without the dialogue open.
          const StatusLineFrb(),
        ],
      );

  /// Which keymap context the focused panel is. Panels with no bindings of
  /// their own resolve to `Global`, which is also the fallback for every other
  /// context, so nothing is lost by the mapping being partial.
  BridgeKeyContext _contextOf(Panel? panel) => switch (panel) {
        Panel.project => BridgeKeyContext.project,
        Panel.viewer => BridgeKeyContext.viewer,
        Panel.timeline => BridgeKeyContext.timeline,
        Panel.effectControls => BridgeKeyContext.effects,
        _ => BridgeKeyContext.global,
      };

  /// The keyboard shortcuts, restored from the shell the port replaced.
  ///
  /// Only the ones whose engine calls exist on this bridge; the rest are on the
  /// menus. A field with focus is left alone, or every letter typed into a
  /// layer name would also be a command.
  KeyEventResult _onKey(LumitState state, LumitUiState ui, KeyEvent event) {
    if (event is! KeyDownEvent && event is! KeyRepeatEvent) {
      return KeyEventResult.ignored;
    }
    // A modal surface is up — a dialogue, or the FX console (K-328): its keys
    // are its own, exactly as the panels' handlers already treat it (K-243).
    // Without this, a keystroke aimed at the console's search box also ran
    // whatever shell command it happened to spell.
    if (lumitModalOpen) return KeyEventResult.ignored;
    // A field with focus keeps its keys, or typing a layer name would also run
    // commands. The focused context's own widget is the `Focus` that
    // `EditableText` builds, not the `EditableText` — so the check has to look
    // up the tree, which is what the previous shell's version missed.
    final focused = FocusManager.instance.primaryFocus?.context;
    if (focused != null &&
        (focused.widget is EditableText ||
            focused.findAncestorWidgetOfExactType<EditableText>() != null)) {
      return KeyEventResult.ignored;
    }
    // A focused house control (a dialog's OK button, a tabbed-to checkbox)
    // keeps its keys the same way a text field does: Enter or Space there
    // presses the control, and must not also run a panel command underneath
    // it (K-319).
    if (FocusManager.instance.primaryFocus is ControlFocusNode) {
      return KeyEventResult.ignored;
    }

    final project = state.project;
    final comp = ui.selectedComp;

    // Which action this chord means is the engine's answer, not a ladder of key
    // comparisons here (K-199). Asked in the *focused panel's* context, because
    // a binding scoped to one panel has to beat the app-wide one while that
    // panel is active — the engine falls back to Global itself, so one call
    // answers both. (This handler runs wherever focus is; the active panel is
    // what the dock last fronted, which is what a user would call "where I am".)
    var action = ui.keymap.actionFor(_contextOf(ui.activePanel.value), event);
    if (action == null) {
      // The Tools context is a context no panel *is* (docs/07 §15 scopes it to
      // the toolbar, not to a pane), so it is asked for separately and only
      // once the focused panel and the app-wide table have both declined.
      // That ordering is what keeps a panel free to claim a letter a tool also
      // uses — `C` cuts a clip in the Timeline and arms the razor everywhere
      // else — without either binding having to know about the other.
      final tool = ui.keymap.actionFor(BridgeKeyContext.tools, event);
      if (tool != null && ui.tools.handleAction(tool)) {
        return KeyEventResult.handled;
      }
      // **Panels** is the other one, and for the same reason: its three
      // bindings are about moving *between* panels, so scoping them to one
      // would make them unreachable from every other. Asked last, so a panel
      // that binds `Ctrl+F` for itself one day would still win where it is
      // focused. The engine falls back to Global from here too, which the
      // first lookup has already covered, so nothing can be dispatched twice.
      action = ui.keymap.actionFor(BridgeKeyContext.panels, event);
      if (action == null) return KeyEventResult.ignored;
    }
    // A tool action can also arrive from the primary lookup, if someone rebinds
    // one into a context a panel is. Same handler either way.
    if (ui.tools.handleAction(action)) return KeyEventResult.handled;

    var handled = true;
    switch (action) {
      case 'edit.redo':
        project?.redo();
        state.notifyDocumentChanged();
      case 'edit.undo':
        project?.undo();
        state.notifyDocumentChanged();
      case 'playback.toggle':
        ui.requestTogglePlay();
      // Shuttle is not built, and J/L have always stepped a frame here. Mapping
      // them onto the step keeps today's keyboard exactly as it is rather than
      // taking two keys away until a shuttle exists to give them back.
      case 'playback.frame.prev' || 'playback.shuttle.reverse':
        ui.stepFrame(-1);
      case 'playback.frame.next' || 'playback.shuttle.forward':
        ui.stepFrame(1);
      case 'playback.comp.start':
        ui.playheadFrame.value = 0;
      case 'playback.comp.end':
        final last = (comp?.durationFrames() ?? 1) - 1;
        ui.playheadFrame.value = last < 0 ? 0 : last;
      case 'layer.retime.enable':
        // Give the selected layer a Retime, or take it away again (docs/04
        // §12). On installs the identity map, so the picture does not move —
        // it just gains a row above Transform to key. Ctrl+Alt+T by default
        // (K-200): AE's own Time Remap chord, and one Windows cannot steal.
        // The Composition menu carries the command too (K-198's lesson).
        final layer = ui.selectedLayer.value;
        if (layer == null) {
          handled = false;
        } else {
          state.toggleRetime(layer);
        }
      // The Viewer's own magnification and preview resolution (docs/07 §2.2,
      // §15). Both are asked for rather than done here: the magnification
      // belongs to the Viewer panel, and the resolution is a number every
      // render request already carries.
      case 'viewer.zoom.in':
        ui.requestViewerZoom(ViewerZoomCommand.zoomIn);
      case 'viewer.zoom.out':
        ui.requestViewerZoom(ViewerZoomCommand.zoomOut);
      case 'viewer.zoom.fit':
        ui.requestViewerZoom(ViewerZoomCommand.fit);
      case 'viewer.res.full':
        ui.setPreviewResolution(PreviewResolution.full);
      case 'viewer.res.half':
        ui.setPreviewResolution(PreviewResolution.half);
      case 'viewer.res.quarter':
        ui.setPreviewResolution(PreviewResolution.quarter);
      // Moving between panels without the mouse (docs/07 §15, "Panels").
      case 'panel.focus.next':
        handled = ui.cyclePanelFocus(1);
      case 'panel.focus.prev':
        handled = ui.cyclePanelFocus(-1);
      case 'panel.search.focus':
        handled = ui.requestPanelSearch();
      case 'console.open':
        // The menu bar owns the console's lists too, so the key asks for it
        // rather than assembling a second one (K-324).
        ui.requestConsole();
      case 'palette.open':
        // The menu bar owns the palette's list of commands, so the key asks
        // for it rather than assembling a second one (docs/07 §12).
        ui.requestPalette();
      case 'layer.duplicate':
        final layer = ui.selectedLayer.value;
        if (layer == null) {
          handled = false;
        } else {
          layer.duplicate();
          state.notifyDocumentChanged();
        }
      case 'layer.precompose':
        // Ctrl+Shift+C asks before it packs (docs/07 §13.4): the dialogue is
        // where the two questions live, and the engine call is one line of it.
        final layers = ui.selectedLayers.value;
        if (comp == null || layers.isEmpty) {
          handled = false;
        } else {
          showPrecomposeDialogFrb(
            context: context,
            comp: comp,
            selectedLayers: layers,
            ui: ui,
            workspace: ui.workspace,
          );
        }
      // The rest of the menu bar's own commands (K-244). Each calls the very
      // function its menu row calls, so there is one implementation of "open a
      // project" rather than a keyboard's copy of one.
      case 'file.new':
        state.newProject();
      case 'file.open':
        openProjectFrb(state);
      case 'file.save.as':
        saveProjectFrb(state, ui, forcePicker: true);
      case 'file.import':
        importFootageFrb(state);
      case 'file.export':
        if (comp == null) {
          handled = false;
        } else {
          exportFrb(context);
        }
      case 'comp.new':
        if (project == null) {
          handled = false;
        } else {
          newCompositionFrb(context, state);
        }
      // Cut, copy and paste (K-300). The same three functions the Edit menu's
      // rows call — the chords had no handler at all before, which is why
      // `Ctrl+C` on a selected layer did nothing while the menu row worked.
      case 'edit.copy':
        handled = copySelectionFrb(ui);
      case 'edit.cut':
        handled = cutSelectionFrb(state, ui);
      case 'edit.paste':
        // Reading the system clipboard is asynchronous, so the chord is taken
        // and the paste lands a frame later rather than being declined here.
        pasteSelectionFrb(state, ui, comp, ui.selectedLayer.value);
      case 'edit.select.all':
        if (comp == null) {
          handled = false;
        } else {
          ui.setSelection(comp.getLayers());
        }
      case 'edit.deselect.all':
        ui.clearSelection();
      case 'app.settings':
        showSettingsWindowFrb(context);
      case 'project.settings':
        if (project == null) {
          handled = false;
        } else {
          showProjectSettingsFrb(context, project);
        }
      case 'file.save':
        // Ctrl+S goes through exactly the same call the File menu's Save does
        // (K-203) — a shortcut with its own path to disk is a second save to
        // keep honest. Without a path yet it opens the picker, which is what
        // Save has always meant on a document that has never been written.
        saveProjectFrb(state, ui);
      // The work area is the span the Viewer previews and the export writes
      // (K-037), so setting its ends from the playhead is a two-key job, not a
      // trip to a menu. A comp that has never had one set reads as the whole
      // comp, so B and N always have something to move.
      case 'workarea.set.start' || 'workarea.set.end':
        if (comp == null) {
          handled = false;
        } else {
          comp.setWorkArea(
            span: workAreaWith(
              comp: comp,
              current: comp.getWorkArea(),
              wanted: ui.playheadFrame.value,
              isStart: action == 'workarea.set.start',
            ),
          );
          state.notifyDocumentChanged();
        }
      // Markers (K-254). `Shift+M` (or AE's numpad `*`) drops a plain cue at
      // the playhead; `Ctrl`+digit sets the numbered one and the bare digit
      // returns to it. The numbered pair is the whole point — the key that
      // marks a moment is the key that goes back to it.
      case 'marker.add':
        if (comp == null) {
          handled = false;
        } else {
          addMarkerFrb(comp, frame: ui.playheadFrame.value);
          state.notifyDocumentChanged();
        }
      case final id when id.startsWith('marker.add.'):
        if (comp == null) {
          handled = false;
        } else {
          addMarkerFrb(
            comp,
            frame: ui.playheadFrame.value,
            label: id.substring('marker.add.'.length),
          );
          state.notifyDocumentChanged();
        }
      case final id when id.startsWith('marker.goto.'):
        // Nothing bound to that digit yet is not a failure to report — it is a
        // key that has not been given a meaning. Left unhandled so it still
        // reaches whatever else wants it.
        final at = comp == null
            ? null
            : markerFrameFrb(comp, id.substring('marker.goto.'.length));
        if (at == null) {
          handled = false;
        } else {
          // Through the scrub, so a digit pressed mid-playback lands where it
          // says rather than being overwritten by the next frame that arrives.
          ui.scrubTo(at);
        }
      case 'edit.delete.selection':
        // A panel holding a finer selection than the layer one gets the key
        // first (K-234) — a selected mask row is what Delete is about, not the
        // layer under it.
        if (ui.deleteClaim?.call() ?? false) {
          break;
        }
        // The whole selection, not just the primary (K-217): with several
        // layers boxed in the Viewer, Delete taking one of them would be a
        // surprise every time.
        final layers = ui.selectedLayers.value;
        if (layers.isEmpty) {
          handled = false;
        } else {
          for (final layer in layers) {
            layer.delete();
          }
          ui.clearSelection();
          state.notifyDocumentChanged();
        }
      // A bound action this shell has no call for yet — the menus carry those.
      // Ignored rather than swallowed, so the key still reaches whatever else
      // wants it.
      default:
        handled = false;
    }
    return handled ? KeyEventResult.handled : KeyEventResult.ignored;
  }
}
