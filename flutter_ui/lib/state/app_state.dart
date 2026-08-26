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
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:lumit_flutter/state/ui_state.dart';
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
String? projectPathFromArgs(List<String> args) {
  for (final a in args) {
    if (a.toLowerCase().endsWith('.lum') && File(a).existsSync()) return a;
  }
  return null;
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
  /// A batch is **one** undo step (K-581): picking six files in the dialogue,
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
        project.importFootage(path: path);
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
