// The Phase-F0 stand-in for the engine-backed application state. In the egui
// frontend this is `AppState` (crates/lumit-ui/src/app_state/), owned by Rust;
// here it is a small ChangeNotifier that answers the chrome's questions and
// records the actions the chrome dispatches, so every menu item, shortcut and
// panel control can be wired now and re-pointed at the bridge in Phase F1
// (docs/flutter-port/03-ARCHITECTURE.md).

import 'dart:io';

import 'package:flutter/foundation.dart';

import '../bridge/bridge.dart';
import 'file_dialogs.dart';

/// One entry in the stub's action log — what a real engine call would have
/// been. The status line surfaces the latest as a notice, so clicking through
/// the chrome shows honest feedback about what is and isn't wired yet.
class StubAction {
  final String action;
  final DateTime at;
  StubAction(this.action) : at = DateTime.now();
}

class AppStateStub extends ChangeNotifier {
  /// The engine bridge, when the `lumit_bridge` library loaded (injected from
  /// `main.dart` via `LumitBridge.tryLoad()`). Null means the app runs on its
  /// F0 placeholders exactly as before — every path here degrades to the
  /// notice-only behaviour when this is null.
  final DocumentBridge? bridge;

  /// The latest document snapshot from the bridge, or null when there is no
  /// bridge. The Project panel renders this when present.
  BridgeSnapshot? snapshot;

  /// File-dialogue seams, defaulting to the real file_selector calls. Tests
  /// inject their own so they never touch a plugin channel (dialogues cannot
  /// open in a widget test).
  final Future<String?> Function() openProjectPicker;
  final Future<String?> Function() saveLocationPicker;
  final Future<List<String>> Function() footagePicker;

  /// Called with the file a project was opened from or saved to, so the
  /// workspace can restore it next launch (wired to `Workspace.rememberProject`
  /// by the shell; null in tests that do not care).
  final void Function(String path)? rememberProject;

  AppStateStub({
    this.bridge,
    Future<String?> Function()? openProjectPicker,
    Future<String?> Function()? saveLocationPicker,
    Future<List<String>> Function()? footagePicker,
    this.rememberProject,
    String? lastProjectPath,
  })  : openProjectPicker = openProjectPicker ?? pickProjectToOpen,
        saveLocationPicker = saveLocationPicker ?? pickProjectSaveLocation,
        footagePicker = footagePicker ?? pickFootage {
    // A live bridge means a live document from the first frame: pull the
    // initial snapshot so the Project panel is populated immediately.
    if (bridge != null) {
      final reply = bridge!.snapshot();
      if (reply.ok) {
        _adoptSnapshot(reply.snapshot);
      } else {
        errorNotice = reply.error;
      }
      _restoreLastProject(lastProjectPath);
    }
  }

  /// On launch with a live bridge, reopen the last project if its file is still
  /// on disk. A missing file is not an error (the project simply moved); a
  /// failed open degrades to a calm status-line notice, never a crash.
  void _restoreLastProject(String? path) {
    if (path == null || bridge == null) return;
    if (!File(path).existsSync()) return;
    final reply = bridge!.openProject(path);
    if (reply.ok) {
      _adoptSnapshot(reply.snapshot);
      notice = 'Project reopened';
    } else {
      notice = 'the last project could not be reopened';
    }
  }

  /// Quiet status-line notice (docs/15 §10 — completion is quiet).
  String? notice;

  /// A genuine error, drawn in the error tint. Kept separate from `notice`
  /// exactly as the Rust side splits them.
  String? errorNotice;

  bool playing = false;
  int previewFrame = 0;
  int previewFrameCount = 0;
  double timelineZoom = 1.0;
  bool timelineGraphMode = false;
  bool snapping = true;
  int? selectedLayer;
  final List<String> openComps = [];
  int beatSensitivity = 50;
  bool canUndo = false;
  bool canRedo = false;

  final List<StubAction> actionLog = [];

  /// Record an engine action the bridge will implement, and say so in the
  /// status line — never silently swallow a click.
  void engine(String action) {
    actionLog.add(StubAction(action));
    notice = '$action — engine bridge arrives in phase F1';
    notifyListeners();
  }

  void togglePlay() {
    playing = !playing;
    notifyListeners();
  }

  void stepFrame(int delta) {
    if (playing) playing = false;
    previewFrame = (previewFrame + delta).clamp(0, previewFrameCount);
    notifyListeners();
  }

  void goToFrame(int frame) {
    if (playing) playing = false;
    previewFrame = frame.clamp(0, previewFrameCount);
    notifyListeners();
  }

  void zoomTimeline(double factor) {
    timelineZoom = (timelineZoom * factor).clamp(1.0, 400.0);
    notifyListeners();
  }

  void zoomTimelineFit() {
    timelineZoom = 1.0;
    notifyListeners();
  }

  void toggleGraphMode() {
    timelineGraphMode = !timelineGraphMode;
    notifyListeners();
  }

  void setNotice(String? n) {
    notice = n;
    notifyListeners();
  }

  // --- Bridge-routed document actions -------------------------------------
  //
  // Each mirrors an egui `AppState` action. With no bridge they fall back to
  // the F0 notice, unchanged, so the placeholder build behaves exactly as
  // before. With a bridge they route to the engine, refresh the held snapshot,
  // and surface any error in the error tint.

  void newProject() {
    if (bridge == null) {
      engine('New project');
      return;
    }
    _applyReply(bridge!.newProject(), 'New project');
  }

  void newComposition() {
    if (bridge == null) {
      engine('New composition');
      return;
    }
    // Bridge v0 has no dialogue yet, so the engine names the comp ("Comp N").
    _applyReply(bridge!.newComposition(''), 'Composition added');
  }

  void undo() {
    if (bridge == null) {
      engine('Undo');
      return;
    }
    _applyReply(bridge!.undo(), 'Undone');
  }

  void redo() {
    if (bridge == null) {
      engine('Redo');
      return;
    }
    _applyReply(bridge!.redo(), 'Redone');
  }

  Future<void> save() async {
    if (bridge == null) {
      engine('Save');
      return;
    }
    // A known path saves in place; without one, Save falls through to a save
    // dialogue — that is the egui behaviour (there is no separate Save As).
    if (snapshot?.path != null) {
      final reply = bridge!.saveProject('');
      _applyReply(reply, 'Project saved');
      if (reply.ok && snapshot?.path != null) rememberProject?.call(snapshot!.path!);
      return;
    }
    final path = await saveLocationPicker();
    if (path == null) return; // cancelled — leave the status line as-is
    final reply = bridge!.saveProject(path);
    _applyReply(reply, 'Project saved');
    if (reply.ok) rememberProject?.call(snapshot?.path ?? path);
  }

  Future<void> openProject() async {
    if (bridge == null) {
      engine('Open project');
      return;
    }
    final path = await openProjectPicker();
    if (path == null) return; // cancelled — leave the status line as-is
    final reply = bridge!.openProject(path);
    _applyReply(reply, 'Project opened');
    if (reply.ok) rememberProject?.call(path);
  }

  Future<void> importFootage() async {
    if (bridge == null) {
      engine('Import footage');
      return;
    }
    final paths = await footagePicker();
    if (paths.isEmpty) return; // cancelled or nothing chosen
    var imported = 0;
    var failed = 0;
    String? lastError;
    for (final path in paths) {
      final reply = bridge!.importFootage(path);
      if (reply.ok) {
        _adoptSnapshot(reply.snapshot);
        imported++;
      } else {
        failed++;
        lastError = reply.error;
      }
    }
    _postImportNotice(imported, failed, lastError);
    notifyListeners();
  }

  /// One calm line for an import: the count of items brought in as the notice,
  /// any failures in the error tint (the status line shows the error when both
  /// are set, so a partial failure is never hidden).
  void _postImportNotice(int imported, int failed, String? lastError) {
    notice = imported == 0
        ? null
        : imported == 1
            ? '1 item imported'
            : '$imported items imported';
    errorNotice = failed == 0
        ? null
        : failed == 1
            ? (lastError ?? '1 item could not be imported')
            : '$failed items could not be imported';
  }

  /// Adopt a snapshot into the held state (undo/redo flags follow it).
  void _adoptSnapshot(BridgeSnapshot? snap) {
    if (snap == null) return;
    snapshot = snap;
    canUndo = snap.canUndo;
    canRedo = snap.canRedo;
  }

  /// Apply a bridge reply: on success refresh the snapshot and post a quiet
  /// confirmation; on failure surface the engine's message in the error tint.
  void _applyReply(BridgeReply reply, String done) {
    if (reply.ok) {
      _adoptSnapshot(reply.snapshot);
      notice = done;
      errorNotice = null;
    } else {
      errorNotice = reply.error;
    }
    notifyListeners();
  }
}
