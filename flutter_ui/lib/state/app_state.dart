// The Phase-F0 stand-in for the engine-backed application state. In the egui
// frontend this is `AppState` (crates/lumit-ui/src/app_state/), owned by Rust;
// here it is a small ChangeNotifier that answers the chrome's questions and
// records the actions the chrome dispatches, so every menu item, shortcut and
// panel control can be wired now and re-pointed at the bridge in Phase F1
// (docs/flutter-port/03-ARCHITECTURE.md).

import 'dart:async';
import 'dart:collection';
import 'dart:convert';
import 'dart:io';

import 'package:flutter/foundation.dart';

import '../bridge/bridge.dart';
import '../panels/preview_source.dart';
import 'file_dialogs.dart';
import 'workspace.dart';

/// The payload carried when a Project-panel footage row is dragged onto the
/// Timeline lane (Flutter's [Draggable]/[DragTarget]): the footage item's id and
/// its name (for the drag feedback label). Shared by the Project and Timeline
/// panels so the drop can place it as a layer through `addFootageLayer`.
class FootageDragData {
  final String itemId;
  final String name;
  const FootageDragData(this.itemId, this.name);
}

/// The payload carried when an effect row from the Effects & presets panel is
/// dragged onto a layer (a Flutter [Draggable]/[DragTarget]): the effect's
/// match name and its label (for the drag feedback). Mirrors the egui
/// drag-an-effect-onto-a-layer gesture (docs/07 §7); the drop applies it through
/// `addEffect`.
class EffectDragData {
  final String effectName;
  final String label;
  const EffectDragData(this.effectName, this.label);
}

/// What a pointer drag/click does in the Viewer — the Dart mirror of the egui
/// `ToolMode` (crates/lumit-ui/src/app_state/mod.rs:366). Additive Section-D
/// state on [AppStateStub.viewerTool].
enum ToolMode { select, hand, shape, pen }

/// The mask shape the Shape tool draws — the Dart mirror of the egui `ShapeKind`
/// (crates/lumit-ui/src/app_state/mod.rs:347). The bridge's `addMask` op takes
/// the lower-case name.
enum ShapeKind {
  rectangle,
  ellipse,
  star;

  /// The op's kind string (`rectangle`/`ellipse`/`star`).
  String get opName => name;

  /// The sentence-case label the egui `ShapeKind::label` uses.
  String get label => switch (this) {
        ShapeKind.rectangle => 'Rectangle',
        ShapeKind.ellipse => 'Ellipse',
        ShapeKind.star => 'Star',
      };
}

/// A preview render scale — the Full/Half/Third/Quarter picker in the transport
/// (the egui resolution picker). [factor] is the multiplier the bridge render
/// call takes (`renderCompFrame`'s `scale`); 1.0 is the comp's own resolution.
enum PreviewScale {
  full(1.0, 'Full'),
  half(0.5, 'Half'),
  third(1.0 / 3.0, 'Third'),
  quarter(0.25, 'Quarter');

  final double factor;
  final String label;
  const PreviewScale(this.factor, this.label);
}

/// A text layer's editable content, held per session because snapshot v5 does
/// not carry text content back (only the setter exists). Seeds an em-dash-free
/// editor once the user has committed, and defaults sensibly before then.
class TextContent {
  final String text;
  final double size;
  final List<double> rgba;
  const TextContent(this.text, this.size, this.rgba);

  /// The unedited default: empty text, 72 pt, opaque white (scene-linear).
  static const initial = TextContent('', 72, [1, 1, 1, 1]);

  TextContent copyWith({String? text, double? size, List<double>? rgba}) =>
      TextContent(text ?? this.text, size ?? this.size, rgba ?? this.rgba);
}

/// A solid layer's editable size, held per session because the snapshot carries
/// a solid's colour (`layer.colour`) but not its pixel size (only the setter
/// exists). The colour seeds from the snapshot; this remembers the size.
class SolidSize {
  final int width;
  final int height;
  const SolidSize(this.width, this.height);
}

/// An armed eyedropper: which effect Colour parameter the next Viewer click
/// samples into. The Dart mirror of the egui `EyedropperTarget` (colour mode
/// only in this slice). Set by the effect-controls dropper button, consumed by
/// the Viewer eyedropper overlay, which samples the shown frame and commits
/// through `setEffectParamColour`, then disarms.
class EyedropperArm {
  final String compId;
  final String layerId;
  final String effectId;
  final String paramName;

  /// The alpha to preserve on commit (the sampled pixel writes RGB only, like
  /// the egui colour eyedropper).
  final double alpha;

  const EyedropperArm({
    required this.compId,
    required this.layerId,
    required this.effectId,
    required this.paramName,
    this.alpha = 1.0,
  });
}

/// One pending export in the Dart-side queue — a `VecDeque` mirror of
/// export_actions.rs. The egui side snapshots the whole document at QUEUE time;
/// the bridge can only snapshot at START time, so a Dart queue item carries the
/// call arguments and the document is snapshotted when the export actually
/// begins (a recorded deviation, docs/06 §7.1 and 05-PARITY-CHECKLIST).
class QueuedExport {
  final String compId;
  final String specJson;
  final String outPath;

  /// The file name shown in the status line while this export runs (the path's
  /// last segment).
  final String name;

  const QueuedExport(this.compId, this.specJson, this.outPath, this.name);
}

/// The video bitrate (bits/second) a size-targeted share export uses (K-037) —
/// a faithful port of `Shell::start_share_export`: the byte budget spread over
/// the duration, with the audio track's share removed first and an 8%
/// container/overhead headroom. Pure so it is unit-tested without a bridge.
///
/// [durationSeconds] is the export span in seconds (the work area when set,
/// else the whole comp), floored at 0.1 s exactly as the egui `.max(0.1)`.
/// A leaner 192 kbps AAC rate is subtracted when [hasAudio], because on a share
/// export every audio bit comes out of the same budget. The result is floored
/// at 100 kbps.
int shareExportBitRate({
  required double targetMb,
  required double durationSeconds,
  required bool hasAudio,
}) {
  const audioBitRate = 192000;
  final duration = durationSeconds < 0.1 ? 0.1 : durationSeconds;
  var bits = targetMb * 1000000.0 * 8.0 * 0.92;
  if (hasAudio) bits -= audioBitRate * duration;
  final bitRate = (bits / duration).toInt();
  return bitRate < 100000 ? 100000 : bitRate;
}

/// One entry in the stub's action log — what a real engine call would have
/// been. The status line surfaces the latest as a notice, so clicking through
/// the chrome shows honest feedback about what is and isn't wired yet.
class StubAction {
  final String action;
  final DateTime at;
  StubAction(this.action) : at = DateTime.now();
}

/// One composition as the Timeline's comp-tab strip reads it: its snapshot item
/// [id] (the id ops address), display [name], and its [comp] detail.
class CompTabInfo {
  final String id;
  final String name;
  final BridgeComp comp;
  const CompTabInfo(this.id, this.name, this.comp);
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

  /// The export Save-location seam (suggested name → chosen path, or null when
  /// cancelled), defaulting to the real file_selector call. Tests inject their
  /// own so the export dialogue and share exports never touch a plugin channel.
  final Future<String?> Function(String suggestedName) exportSaveLocationPicker;

  /// The `.lumfx` preset open seam (chosen path, or null when cancelled),
  /// defaulting to the real file_selector call. Tests inject their own.
  final Future<String?> Function() presetOpenPicker;

  /// The `.lumfx` preset save-location seam (suggested name → chosen path, or
  /// null when cancelled), defaulting to the real file_selector call.
  final Future<String?> Function(String suggestedName) presetSaveLocationPicker;

  /// Called with the file a project was opened from or saved to, so the
  /// workspace can restore it next launch (wired to `Workspace.rememberProject`
  /// by the shell; null in tests that do not care).
  final void Function(String path)? rememberProject;

  /// Persist the per-project session (open comps, front comp, playhead,
  /// selection) for a project path — wired to `Workspace.rememberSession` by
  /// the shell; null leaves session state session-only.
  final void Function(String path, SavedSession session)? rememberSession;

  /// Read a stored session for a project path (wired to `Workspace.sessionFor`
  /// by the shell), applied after a project opens. Null means no restore.
  final SavedSession? Function(String path)? sessionFor;

  /// The autosave interval — how often [autosaveTick] writes a rotating copy
  /// when the document is dirty. Defaults to the egui `AUTOSAVE_INTERVAL_SECS`
  /// (5 min). The shell points this at Settings → General.
  Duration autosaveInterval;

  /// How many rotating autosaves to keep (egui `AUTOSAVE_KEEP`); the oldest
  /// falls off. The shell points this at Settings → General.
  int autosaveKeep;

  /// Builds the off-thread frame renderer the [PreviewSource] uses (the perf
  /// pass render isolate). The shell passes a factory that spawns the worker
  /// isolate when the real `lumit_bridge` library is loaded; left null (tests
  /// and the placeholder build) the PreviewSource renders inline on the UI
  /// isolate exactly as before — so widget tests stay deterministic.
  final FrameRenderer? Function(AppStateStub app)? previewRendererFactory;

  AppStateStub({
    this.bridge,
    this.previewRendererFactory,
    Future<String?> Function()? openProjectPicker,
    Future<String?> Function()? saveLocationPicker,
    Future<List<String>> Function()? footagePicker,
    Future<String?> Function(String suggestedName)? exportSaveLocationPicker,
    Future<String?> Function()? presetOpenPicker,
    Future<String?> Function(String suggestedName)? presetSaveLocationPicker,
    this.rememberProject,
    this.rememberSession,
    this.sessionFor,
    this.autosaveInterval = const Duration(minutes: 5),
    this.autosaveKeep = 3,
    String? lastProjectPath,
  })  : openProjectPicker = openProjectPicker ?? pickProjectToOpen,
        saveLocationPicker = saveLocationPicker ?? pickProjectSaveLocation,
        footagePicker = footagePicker ?? pickFootage,
        exportSaveLocationPicker =
            exportSaveLocationPicker ?? pickExportSaveLocation,
        presetOpenPicker = presetOpenPicker ?? pickPresetToOpen,
        presetSaveLocationPicker =
            presetSaveLocationPicker ?? pickPresetSaveLocation {
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
      _applySessionFor(path);
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

  /// The playhead frame as a dedicated fine-grained notifier (the perf pass,
  /// K-176). Pure playhead motion — a scrub, a playback tick — fires THIS and
  /// not the big [notifyListeners], so only the widgets that genuinely track
  /// the playhead per frame rebuild (the Viewer transport readout, the Timeline
  /// playhead line and comp-tab clock, the Scopes/PreviewSource frame source,
  /// the graph readout). Layer rows, the Project/Hierarchy panels and the
  /// effect controls keep listening to the app notifier, which now fires only
  /// on document/selection/notice changes — so they no longer rebuild at frame
  /// rate during a scrub or playback. [previewFrame] mirrors this value for the
  /// many event handlers that read it directly.
  final ValueNotifier<int> playheadFrame = ValueNotifier<int>(0);

  /// Move the playhead: update the plain [previewFrame] field the event
  /// handlers read and fire the fine-grained [playheadFrame] notifier. Never
  /// touches the big notifier — callers decide whether a document/transport
  /// change also warrants [notifyListeners].
  void _setPlayhead(int frame) {
    previewFrame = frame;
    playheadFrame.value = frame;
  }
  double timelineZoom = 1.0;
  bool timelineGraphMode = false;
  bool snapping = true;

  /// The selected layer, by its snapshot layer id (was an int index in F0; the
  /// Timeline selects by the engine's stable layer id so ops address the right
  /// layer). Null when nothing is selected.
  String? selectedLayer;

  /// The selected Project-panel item, by its snapshot item id. Drives the row
  /// highlight; null when nothing is selected there.
  String? selectedProjectItem;

  /// Which composition the Timeline/Viewer front, by snapshot item id. Null
  /// means "the first composition in the snapshot" — the [frontComp] fallback.
  String? frontCompId;

  /// Transform values the user has committed this session, keyed
  /// `"$layerId/$property"`. Snapshot v2 does not carry current transform
  /// values (only the setter exists), so the Effect controls panel shows these
  /// — and an em-dash before any edit — until snapshot v3 delivers read-back.
  /// Additive F4 session state.
  final Map<String, double> transformEdits = {};

  /// The value the user set for [layerId]'s [property] this session, or null if
  /// it has not been edited yet (draw an em-dash in that case).
  double? transformEditAt(String layerId, String property) =>
      transformEdits['$layerId/$property'];

  final List<String> openComps = [];
  int beatSensitivity = 50;
  bool canUndo = false;
  bool canRedo = false;

  // --- Keyframe clipboard seam (egui note 2.2) ----------------------------
  //
  // Ctrl+C/Ctrl+V on the Timeline copy the selected lane keys and paste them at
  // the playhead. The shell owns the key handling (a different agent's file), so
  // it calls [copySelectedKeyframes]/[pasteKeyframes] here; the Timeline body,
  // which owns the lane selection, installs the two handlers and holds the
  // copied payload in [keyframeClipboard] (kept on the app state so it survives
  // the body being rebuilt). Additive: with no handler installed both are quiet
  // no-ops, so the placeholder build is unaffected.

  /// The copied keyframe payload (an opaque JSON string the Timeline produces),
  /// or null before anything is copied.
  String? keyframeClipboard;

  /// Installed by the Timeline body: copies the current lane selection into
  /// [keyframeClipboard].
  void Function()? copyKeyframesHandler;

  /// Installed by the Timeline body: pastes [keyframeClipboard] at the playhead.
  void Function()? pasteKeyframesHandler;

  /// Copy the selected lane keys (the shell's Ctrl+C) — a no-op when the
  /// Timeline is not mounted or nothing is selected.
  void copySelectedKeyframes() => copyKeyframesHandler?.call();

  /// Paste the copied keys at the playhead (the shell's Ctrl+V) — a no-op with
  /// an empty clipboard or no mounted Timeline.
  void pasteKeyframes() => pasteKeyframesHandler?.call();

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
    final wasPlaying = playing;
    playing = false;
    _setPlayhead((previewFrame + delta).clamp(0, previewFrameCount));
    _scheduleSessionPersist();
    // Only the transport-state change needs the big notifier; the frame move
    // itself rides [playheadFrame], so layer rows do not rebuild.
    if (wasPlaying) notifyListeners();
  }

  void goToFrame(int frame) {
    final wasPlaying = playing;
    playing = false;
    _setPlayhead(frame.clamp(0, previewFrameCount));
    _scheduleSessionPersist();
    if (wasPlaying) notifyListeners();
  }

  /// Move the playhead during playback WITHOUT stopping (the Viewer's transport
  /// ticker drives this). Unlike [goToFrame] it leaves `playing` set, so the
  /// loop keeps running. Additive F2 seam. The hottest path: it fires only the
  /// fine-grained [playheadFrame] notifier and never persists — no disk write
  /// during continuous playback.
  void advancePlayback(int frame) {
    _setPlayhead(frame);
  }

  /// The Viewer's CPU frame source (phase F2), shared with the Scopes panel so
  /// both read the same decoded pixels. Created lazily on first use; harmless
  /// without a bridge (it simply never resolves a frame). Single-layer preview
  /// until the compositor is extracted from the egui crate.
  PreviewSource? _previewSource;
  PreviewSource get previewSource =>
      _previewSource ??= PreviewSource(this, renderer: previewRendererFactory?.call(this));

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

  /// Select a layer by its snapshot layer id (the Hierarchy row click), or null
  /// to clear. The Effect controls panel reads [selectedLayer].
  void selectLayer(String? id) {
    if (selectedLayer == id) return;
    selectedLayer = id;
    _scheduleSessionPersist();
    notifyListeners();
  }

  /// Select a Project-panel item by its snapshot item id (a row click), or null
  /// to clear. Drives the row highlight.
  void selectProjectItem(String? id) {
    if (selectedProjectItem == id) return;
    selectedProjectItem = id;
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
    // Closing the current project: flush its pending session before it goes.
    flushPendingSession();
    _applyReply(bridge!.newProject(), 'New project');
  }

  /// Create a composition. [name] carries the name typed in the New
  /// composition dialogue (F4); empty lets the engine name it ("Comp N"). Size,
  /// frame rate and duration are not yet wired — the bridge has no
  /// comp-settings op, so the dialogue collects them but only the name reaches
  /// the engine for now (see the dialogue's pending note).
  void newComposition([String name = '']) {
    if (bridge == null) {
      engine('New composition');
      return;
    }
    _applyReply(bridge!.newComposition(name), 'Composition added');
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
      if (reply.ok) {
        _dirtySinceSave = false;
        if (snapshot?.path != null) rememberProject?.call(snapshot!.path!);
      }
      return;
    }
    final path = await saveLocationPicker();
    if (path == null) return; // cancelled — leave the status line as-is
    final reply = bridge!.saveProject(path);
    _applyReply(reply, 'Project saved');
    if (reply.ok) {
      _dirtySinceSave = false;
      rememberProject?.call(snapshot?.path ?? path);
    }
  }

  Future<void> openProject() async {
    if (bridge == null) {
      engine('Open project');
      return;
    }
    final path = await openProjectPicker();
    if (path == null) return; // cancelled — leave the status line as-is
    // Closing the current project: flush its pending session before it goes.
    flushPendingSession();
    final reply = bridge!.openProject(path);
    _applyReply(reply, 'Project opened');
    if (reply.ok) {
      _dirtySinceSave = false;
      _applySessionFor(path);
      rememberProject?.call(path);
    }
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

  /// Every composition in the current snapshot, top-first (nested comps are
  /// flattened in, after their parent). The Timeline's comp-tab strip renders
  /// this; empty when there is no bridge/snapshot or no composition yet.
  List<CompTabInfo> get compositions {
    final snap = snapshot;
    if (snap == null) return const [];
    final out = <CompTabInfo>[];
    void walk(List<BridgeItem> items) {
      for (final item in items) {
        if (item.kind == BridgeItemKind.composition && item.comp != null) {
          out.add(CompTabInfo(item.id, item.name, item.comp!));
        }
        walk(item.children);
      }
    }

    walk(snap.items);
    return out;
  }

  /// The active comp tab: [frontCompId] when it still resolves, else the first
  /// composition in the snapshot. Null when there is no composition.
  CompTabInfo? get _frontTab {
    final comps = compositions;
    if (comps.isEmpty) return null;
    final id = frontCompId;
    if (id != null) {
      for (final c in comps) {
        if (c.id == id) return c;
      }
    }
    return comps.first;
  }

  /// The active comp the Viewer and Timeline read. Honours [frontCompId] and
  /// falls back to the first composition. Null when there is no composition.
  BridgeComp? get frontComp => _frontTab?.comp;

  /// The snapshot item id of the [frontComp] — the id the Timeline passes to the
  /// layer/marker ops. Null when there is no composition.
  String? get frontCompIdResolved => _frontTab?.id;

  /// Front the composition with snapshot item [id] (a comp-tab click). Also
  /// re-syncs the playhead range to that comp's frame count.
  void frontCompSelect(String id) {
    if (frontCompId == id) return;
    frontCompId = id;
    previewFrameCount = frontComp?.frameCount ?? previewFrameCount;
    _setPlayhead(previewFrame.clamp(0, previewFrameCount));
    _scheduleSessionPersist();
    notifyListeners();
  }

  // --- Snapshot-v2 op pass-throughs ---------------------------------------
  //
  // The Timeline and editor panels drive these; each routes to the engine,
  // refreshes the held snapshot and surfaces any error in the error tint. With
  // no bridge they are quiet no-ops (the placeholder build has no document).

  /// Flip a layer's switch (`visible`, `audible`, `locked`, `solo`,
  /// `motion_blur`, `fx`, `three_d`, `collapse`).
  void setLayerSwitch(
      String compId, String layerId, String switchName, bool value) {
    final b = bridge;
    if (b == null) return;
    _applyOp(b.setLayerSwitch(compId, layerId, switchName, value));
  }

  /// Edit a layer's span at [frame] (`move_in`, `move_out`, `trim_in`,
  /// `trim_out`).
  void editLayerSpan(String compId, String layerId, String edit, int frame) {
    final b = bridge;
    if (b == null) return;
    _applyOp(b.editLayerSpan(compId, layerId, edit, frame));
  }

  /// Set one transform property to a static [value] (snake_case `TransformProp`
  /// name, e.g. `position_x`, `opacity`).
  void setTransform(
      String compId, String layerId, String property, double value) {
    // Remember the value so the Effect controls panel can show it back (the
    // snapshot does not carry current transform values yet — see [transformEdits]).
    transformEdits['$layerId/$property'] = value;
    final b = bridge;
    if (b == null) {
      notifyListeners();
      return;
    }
    _applyOp(b.setTransform(compId, layerId, property, value));
  }

  /// Drop a user marker on the composition timeline at [frame].
  void addMarker(String compId, int frame) {
    final b = bridge;
    if (b == null) return;
    _applyOp(b.addMarker(compId, frame));
  }

  // --- Bridge v0.3 op pass-throughs ---------------------------------------
  //
  // Each routes to the engine, refreshes the held snapshot and surfaces any
  // error in the error tint. With no bridge they are quiet no-ops.

  /// Add a Solid layer to [compId].
  void addSolidLayer(String compId) => _bridgeOp((b) => b.addSolidLayer(compId));

  /// Add a Text layer to [compId].
  void addTextLayer(String compId) => _bridgeOp((b) => b.addTextLayer(compId));

  /// Add a Camera layer to [compId].
  void addCameraLayer(String compId) =>
      _bridgeOp((b) => b.addCameraLayer(compId));

  /// Add an Adjustment layer to [compId].
  void addAdjustmentLayer(String compId) =>
      _bridgeOp((b) => b.addAdjustmentLayer(compId));

  /// Add an (empty) Sequence layer to [compId].
  void addSequenceLayer(String compId) =>
      _bridgeOp((b) => b.addSequenceLayer(compId));

  /// Place the project footage item [itemId] into [compId] as a new Footage
  /// layer (top of the stack).
  void addFootageLayer(String compId, String itemId) =>
      _bridgeOp((b) => b.addFootageLayer(compId, itemId));

  /// Place the footage item [itemId] into the front composition as a new layer
  /// (the Project panel's double-click / drag-drop). No front comp surfaces a
  /// calm notice rather than silently doing nothing.
  void addFootageToFrontComp(String itemId) {
    final compId = frontCompIdResolved;
    if (compId == null) {
      setNotice('Open a composition to place footage into');
      return;
    }
    addFootageLayer(compId, itemId);
  }

  /// Reorder a layer within its composition to [newIndex] (0 = top) — the
  /// Timeline drag-reorder.
  void reorderLayer(String compId, String layerId, int newIndex) =>
      _bridgeOp((b) => b.reorderLayer(compId, layerId, newIndex));

  /// Delete a layer from its composition.
  void deleteLayer(String compId, String layerId) =>
      _bridgeOp((b) => b.deleteLayer(compId, layerId));

  /// Duplicate a layer (a copy above the original).
  void duplicateLayer(String compId, String layerId) =>
      _bridgeOp((b) => b.duplicateLayer(compId, layerId));

  /// Edit a composition's settings as one undo step.
  void setCompSettings(String compId, String name, int width, int height,
          int fpsNum, int fpsDen, int durationFrames) =>
      _bridgeOp((b) => b.setCompSettings(
          compId, name, width, height, fpsNum, fpsDen, durationFrames));

  /// The stopwatch: toggle a transform property's animation at [frame].
  void togglePropertyAnimated(
          String compId, String layerId, String property, int frame) =>
      _bridgeOp((b) => b.togglePropertyAnimated(compId, layerId, property, frame));

  /// Insert or replace a transform keyframe at [frame] with [value].
  void addKeyframe(
      String compId, String layerId, String property, int frame, double value) {
    transformEdits['$layerId/$property'] = value;
    _bridgeOp((b) => b.addKeyframe(compId, layerId, property, frame, value));
  }

  /// Remove the transform keyframe at [frame].
  void removeKeyframe(String compId, String layerId, String property, int frame) =>
      _bridgeOp((b) => b.removeKeyframe(compId, layerId, property, frame));

  /// Slide the transform keyframes at comp [frames] by [delta] frames.
  void shiftKeyframes(String compId, String layerId, String property,
          List<int> frames, int delta) =>
      _bridgeOp((b) => b.shiftKeyframes(compId, layerId, property, frames, delta));

  /// Set one work-area edge to the playhead [frame] ([isOut] picks the out
  /// edge).
  void setWorkAreaEdge(String compId, int frame, bool isOut) =>
      _bridgeOp((b) => b.setWorkAreaEdge(compId, frame, isOut));

  /// The B key: set the work-area IN edge to the current playhead on the front
  /// comp. A convenience over [setWorkAreaEdge] resolving the comp + playhead,
  /// so the shell's B shortcut drives the real op rather than the F0 notice.
  void workAreaInAtPlayhead() {
    final id = frontCompIdResolved;
    if (id == null) return;
    setWorkAreaEdge(id, previewFrame, false);
  }

  /// The N key: set the work-area OUT edge to the current playhead on the front
  /// comp (the sibling of [workAreaInAtPlayhead]).
  void workAreaOutAtPlayhead() {
    final id = frontCompIdResolved;
    if (id == null) return;
    setWorkAreaEdge(id, previewFrame, true);
  }

  /// The built-in effect registry (empty without a bridge).
  List<BridgeEffectInfo> listEffects() => bridge?.listEffects() ?? const [];

  /// Apply a built-in effect (by its match name) to a layer.
  void addEffect(String compId, String layerId, String effectName) =>
      _bridgeOp((b) => b.addEffect(compId, layerId, effectName));

  /// Remove an effect instance from a layer.
  void removeEffect(String compId, String layerId, String effectId) =>
      _bridgeOp((b) => b.removeEffect(compId, layerId, effectId));

  /// Enable or bypass an effect instance.
  void setEffectEnabled(
          String compId, String layerId, String effectId, bool enabled) =>
      _bridgeOp((b) => b.setEffectEnabled(compId, layerId, effectId, enabled));

  /// Set a scalar (Float) effect parameter to a static [value].
  void setEffectParamScalar(String compId, String layerId, String effectId,
          String paramName, double value) =>
      _bridgeOp((b) =>
          b.setEffectParamScalar(compId, layerId, effectId, paramName, value));

  /// Set a Colour effect parameter to a static scene-linear RGBA.
  void setEffectParamColour(String compId, String layerId, String effectId,
          String paramName, double r, double g, double b, double a) =>
      _bridgeOp((bridge) => bridge.setEffectParamColour(
          compId, layerId, effectId, paramName, r, g, b, a));

  // --- Bridge v0.4 op pass-throughs ---------------------------------------

  /// Set the interpolation of the keyframe nearest [frame] on a transform
  /// [property] (`Hold`/`Linear`/`Bezier`; the speed/influence pairs apply only
  /// to a `Bezier` side).
  void setKeyframeInterp(
          String compId,
          String layerId,
          String property,
          int frame,
          String interpIn,
          String interpOut,
          {double speedIn = 0,
          double influenceIn = 1.0 / 3.0,
          double speedOut = 0,
          double influenceOut = 1.0 / 3.0}) =>
      _bridgeOp((b) => b.setKeyframeInterp(compId, layerId, property, frame,
          interpIn, interpOut, speedIn, influenceIn, speedOut, influenceOut));

  /// Enable or disable a footage layer's Retime (the Time stopwatch).
  void setRetimeEnabled(String compId, String layerId, bool enabled) =>
      _bridgeOp((b) => b.setRetimeEnabled(compId, layerId, enabled));

  /// Set a footage layer's constant playback speed (percent; 100 clears it).
  void setRetimeSpeed(String compId, String layerId, double speedPercent) =>
      _bridgeOp((b) => b.setRetimeSpeed(compId, layerId, speedPercent));

  /// Set the ease of the Retime segment at [frame].
  void setSegmentPreset(String compId, String layerId, int frame, String ease) =>
      _bridgeOp((b) => b.setSegmentPreset(compId, layerId, frame, ease));

  /// Convert the Map segment at [frame] to a Rate segment.
  void segmentToRate(String compId, String layerId, int frame) =>
      _bridgeOp((b) => b.segmentToRate(compId, layerId, frame));

  /// →Rate on the Retime segment under [frame], surfacing the fit outcome as a
  /// calm status notice the way the egui speed lens does (docs/04-RETIMING.md
  /// §5.2). The engine reports a fit `drift` in its reply, but the typed
  /// [BridgeReply]/[BridgeSnapshot] do not carry that field (it is dropped in
  /// the bridge's snapshot decode, out of this slice's scope), so we post the
  /// clean-fit confirmation without the millisecond figure — the numeric drift
  /// badge stays a named remainder in the parity checklist.
  void convertSegmentToRate(String compId, String layerId, int frame) {
    final b = bridge;
    if (b == null) return;
    final reply = b.segmentToRate(compId, layerId, frame);
    if (reply.ok) {
      _adoptSnapshot(reply.snapshot);
      // The egui wording: the fit drift surfaces as a quiet notice.
      final drift = reply.driftSeconds;
      notice = drift == null
          ? 'Converted to rate'
          : 'fitted, ${(drift * 1000).round()} ms drift';
      errorNotice = null;
    } else {
      errorNotice = reply.error;
    }
    notifyListeners();
  }

  /// Move the value-lens Retime boundary at [index] to comp [frame].
  void dragBoundary(String compId, String layerId, int index, int frame) =>
      _bridgeOp((b) => b.dragBoundary(compId, layerId, index, frame));

  /// The blend-mode registry (empty without a bridge).
  List<BridgeBlendMode> listBlendModes() =>
      bridge?.listBlendModes() ?? const [];

  /// Set a layer's blend mode (the serde variant name).
  void setBlendMode(String compId, String layerId, String mode) =>
      _bridgeOp((b) => b.setBlendMode(compId, layerId, mode));

  /// Point a layer at another as its matte, or clear it when [source] is empty.
  void setMatte(String compId, String layerId, String source, String channel,
          bool inverted) =>
      _bridgeOp((b) => b.setMatte(compId, layerId, source, channel, inverted));

  /// Point a layer at another as its transform parent, or clear it when
  /// [parent] is empty.
  void setParent(String compId, String layerId, String parent) =>
      _bridgeOp((b) => b.setParent(compId, layerId, parent));

  /// Set the comp's motion-blur master.
  void setMotionBlur(String compId, bool enabled, double shutterAngle,
          double shutterPhase, int samples) =>
      _bridgeOp((b) =>
          b.setMotionBlur(compId, enabled, shutterAngle, shutterPhase, samples));

  /// Add a starter mask shape (`rectangle`/`ellipse`/`star`) to a layer.
  void addMask(String compId, String layerId, String kind) =>
      _bridgeOp((b) => b.addMask(compId, layerId, kind));

  /// Add a starter mask shape to the currently selected layer of the front
  /// comp (the menu-bar / palette Add-mask path). A quiet error when there is
  /// no selected layer (or no front comp) — never a crash.
  void addMaskToSelected(String kind) {
    final compId = frontCompIdResolved;
    final layerId = selectedLayer;
    if (compId == null || layerId == null) {
      errorNotice = 'select a layer to add a mask';
      notifyListeners();
      return;
    }
    addMask(compId, layerId, kind);
  }

  // --- Bridge v0.5 op pass-throughs ---------------------------------------
  //
  // The v0.5 ops live on the [EditOpsBridge] capability interface (kept off
  // [DocumentBridge] so the older fakes need no change). Each routes through
  // [_editOp], refreshing the snapshot and surfacing any error on the error
  // tint; with no bridge they are quiet no-ops, and with a live library too old
  // to carry the capability they surface one calm notice.

  /// The bridge's edit-ops capability, or null when there is no bridge or the
  /// loaded library predates it.
  EditOpsBridge? get editOps {
    final b = bridge;
    return b is EditOpsBridge ? b as EditOpsBridge : null;
  }

  /// Route an edit op through the capability, applying its reply. A missing
  /// capability (an older library) is one calm notice, never a crash.
  void _editOp(BridgeReply Function(EditOpsBridge e) op) {
    final b = bridge;
    if (b == null) return; // placeholder build — a quiet no-op, as before
    final e = editOps;
    if (e == null) {
      errorNotice = 'this engine build is missing the edit ops';
      notifyListeners();
      return;
    }
    _applyOp(op(e));
  }

  // Razor (sequence layers) — resolves the front comp, selected layer and
  // playhead the way the egui razor does. A non-sequence layer is refused
  // calmly by the engine.

  /// Cut the selected Sequence layer's clip at the playhead into two.
  void cutClipAtPlayhead() {
    final compId = frontCompIdResolved;
    final layerId = selectedLayer;
    if (compId == null || layerId == null) {
      setNotice('Select a sequence layer to cut');
      return;
    }
    _editOp((e) => e.cutClipAtPlayhead(compId, layerId, previewFrame));
  }

  /// Delete the clip under the playhead in the selected Sequence layer.
  void deleteClipAtPlayhead() {
    final compId = frontCompIdResolved;
    final layerId = selectedLayer;
    if (compId == null || layerId == null) {
      setNotice('Select a sequence layer');
      return;
    }
    _editOp((e) => e.deleteClipAtPlayhead(compId, layerId, previewFrame));
  }

  // Beats — on the front comp.

  /// Detect beat markers for the front comp ([sensitivity] 0..100).
  void detectBeats(int sensitivity) {
    final compId = frontCompIdResolved;
    if (compId == null) {
      setNotice('Open a composition to detect beats');
      return;
    }
    _editOp((e) => e.detectBeats(compId, sensitivity));
  }

  /// Remove the detected Beat markers from the front comp.
  void clearBeatMarkers() {
    final compId = frontCompIdResolved;
    if (compId == null) return;
    _editOp((e) => e.clearBeatMarkers(compId));
  }

  // Project-item ops.

  /// Delete a project item.
  void deleteItem(String itemId) => _editOp((e) => e.deleteItem(itemId));

  /// Rename a project item.
  void renameItem(String itemId, String name) =>
      _editOp((e) => e.renameItem(itemId, name));

  /// Move a project item back to the panel root.
  void moveToRoot(String itemId) => _editOp((e) => e.moveToRoot(itemId));

  /// Relink a missing footage item (and same-folder missing siblings) at [path].
  void relink(String itemId, String path) =>
      _editOp((e) => e.relink(itemId, path));

  // Layer-identity ops.

  /// Rename a layer.
  void renameLayer(String compId, String layerId, String name) =>
      _editOp((e) => e.renameLayer(compId, layerId, name));

  /// Convert the selected footage layer into a Sequence layer, in place.
  void convertToSequenced(String compId, String layerId) =>
      _editOp((e) => e.convertToSequenced(compId, layerId));

  /// Trim the selected retimed footage layer to where its source runs out.
  void trimToSourceEnd(String compId, String layerId) =>
      _editOp((e) => e.trimToSourceEnd(compId, layerId));

  // Retime setters.

  /// Set a footage layer's Retime reverse policy.
  void setRetimeReverse(String compId, String layerId, bool reverse) =>
      _editOp((e) => e.setRetimeReverse(compId, layerId, reverse));

  /// Set a footage layer's frame interpolation (`nearest`/`blend`/`flow`).
  void setRetimeInterpolation(String compId, String layerId, String interp) =>
      _editOp((e) => e.setRetimeInterpolation(compId, layerId, interp));

  // Asset-property ops.

  /// Set a text layer's content (`text`, `size` in points, scene-linear RGBA
  /// fill).
  void setTextContent(String compId, String layerId, String text, double size,
          double r, double g, double b, double a) =>
      _editOp((e) => e.setTextContent(compId, layerId, text, size, r, g, b, a));

  /// Recolour and resize a solid layer's backing asset.
  void setSolid(String compId, String layerId, double r, double g, double b,
          double a, int width, int height) =>
      _editOp((e) => e.setSolid(compId, layerId, r, g, b, a, width, height));

  /// Set a camera layer's zoom (pixels).
  void setCameraZoom(String compId, String layerId, double zoom) =>
      _editOp((e) => e.setCameraZoom(compId, layerId, zoom));

  // Extra effect-param setters + reorder + the linked-keyframe batch.

  /// Set an enum (`Choice`) effect parameter to an option [index].
  void setEffectParamChoice(String compId, String layerId, String effectId,
          String paramName, int index) =>
      _editOp((e) =>
          e.setEffectParamChoice(compId, layerId, effectId, paramName, index));

  /// Set a `Bool` effect parameter.
  void setEffectParamBool(String compId, String layerId, String effectId,
          String paramName, bool value) =>
      _editOp((e) =>
          e.setEffectParamBool(compId, layerId, effectId, paramName, value));

  /// Set a `Seed` effect parameter.
  void setEffectParamSeed(String compId, String layerId, String effectId,
          String paramName, int seed) =>
      _editOp((e) =>
          e.setEffectParamSeed(compId, layerId, effectId, paramName, seed));

  /// Set a `Point` effect parameter to a static `(x, y)`.
  void setEffectParamPoint(String compId, String layerId, String effectId,
          String paramName, double x, double y) =>
      _editOp((e) =>
          e.setEffectParamPoint(compId, layerId, effectId, paramName, x, y));

  /// Reorder an effect within a layer's stack to [newIndex].
  void reorderEffect(String compId, String layerId, String effectId,
          int newIndex) =>
      _editOp((e) => e.reorderEffect(compId, layerId, effectId, newIndex));

  /// Apply several transform-keyframe edits as one undo step (the linked x/y
  /// pair). [opsJson] is a JSON array of `{property, action, frame, value?}`.
  void applyKeyframeBatch(String compId, String layerId, String opsJson) =>
      _editOp((e) => e.applyKeyframeBatch(compId, layerId, opsJson));

  // Bridge v0.9 pass-throughs: mask geometry, effect-param keyframes, effect
  // presets, and the realtime tier readout.

  /// Add a mask built from a drawn drag rect (`rectangle`/`ellipse`/`star`) at
  /// `(x, y)` sized `w`×`h` in comp pixels — the geometry-carrying Shape-tool
  /// commit (the drawn size/position is honoured, unlike [addMask]).
  void addMaskGeometry(String compId, String layerId, String kind, double x,
          double y, double w, double h) =>
      _editOp((e) => e.addMaskGeometry(compId, layerId, kind, x, y, w, h));

  /// The effect-param stopwatch: toggle keyframing on `(effectId, paramName,
  /// channel)` at the playhead. [channel] is 0 for a scalar, 0/1 for a point,
  /// 0..3 for a colour.
  void toggleEffectParamAnimated(String compId, String layerId, String effectId,
          String paramName, int channel, int frame) =>
      _editOp((e) => e.toggleEffectParamAnimated(
          compId, layerId, effectId, paramName, channel, frame));

  /// Insert or replace an effect-param keyframe at the playhead with [value].
  void addEffectParamKeyframe(String compId, String layerId, String effectId,
          String paramName, int channel, int frame, double value) =>
      _editOp((e) => e.addEffectParamKeyframe(
          compId, layerId, effectId, paramName, channel, frame, value));

  /// Remove the effect-param keyframe at the playhead.
  void removeEffectParamKeyframe(String compId, String layerId, String effectId,
          String paramName, int channel, int frame) =>
      _editOp((e) => e.removeEffectParamKeyframe(
          compId, layerId, effectId, paramName, channel, frame));

  /// Slide the effect-param keyframes at comp [framesJson] by [delta] frames.
  void shiftEffectParamKeyframes(String compId, String layerId, String effectId,
          String paramName, int channel, String framesJson, int delta) =>
      _editOp((e) => e.shiftEffectParamKeyframes(
          compId, layerId, effectId, paramName, channel, framesJson, delta));

  /// Set the interpolation of the effect-param keyframe nearest the playhead.
  void setEffectParamKeyframeInterp(
          String compId,
          String layerId,
          String effectId,
          String paramName,
          int channel,
          int frame,
          String interpIn,
          String interpOut,
          double speedIn,
          double influenceIn,
          double speedOut,
          double influenceOut) =>
      _editOp((e) => e.setEffectParamKeyframeInterp(compId, layerId, effectId,
          paramName, channel, frame, interpIn, interpOut, speedIn, influenceIn,
          speedOut, influenceOut));

  /// Load a `.lumfx` preset ([text] read from a file) onto a layer, appending
  /// its effects with fresh ids as one undo step.
  void loadEffectPreset(String compId, String layerId, String text) =>
      _editOp((e) => e.loadEffectPreset(compId, layerId, text));

  /// Serialise a layer's effect stack to a `.lumfx` JSON string (the Dart side
  /// writes it to a file). Null with no bridge or an older library.
  String? saveEffectPresetJson(String compId, String layerId, String name) {
    final b = bridge;
    if (b is PresetJsonBridge) {
      return (b as PresetJsonBridge).saveEffectPresetJson(compId, layerId, name);
    }
    return null;
  }

  /// Save the selected layer's effect stack as a `.lumfx` preset: serialise it
  /// through the bridge (byte-compatible with `lumit-ui`'s `preset.rs`), ask
  /// where to save, and write the file — the Effects & presets panel's Save
  /// preset action. A calm notice reports the outcome; a quiet error when there
  /// is no selected layer or the library predates presets.
  Future<void> saveSelectedEffectPreset() async {
    final compId = frontCompIdResolved;
    final layerId = selectedLayer;
    if (compId == null || layerId == null) {
      errorNotice = 'select a layer to save its effects';
      notifyListeners();
      return;
    }
    // Name the preset after the layer (the egui default), stripped of a
    // path-hostile character or two.
    final layerName = _findLayer(snapshot!, layerId)?.name ?? 'preset';
    final json = saveEffectPresetJson(compId, layerId, layerName);
    if (json == null) {
      errorNotice = 'this engine build cannot save effect presets';
      notifyListeners();
      return;
    }
    final suggested = '$layerName.lumfx';
    final path = await presetSaveLocationPicker(suggested);
    if (path == null) return; // cancelled — leave the status line as-is
    try {
      await File(path).writeAsString(json);
      notice = 'preset saved';
      errorNotice = null;
    } catch (e) {
      errorNotice = 'could not write the preset';
    }
    notifyListeners();
  }

  /// Load a `.lumfx` preset onto the selected layer: ask for a file, read it,
  /// and append its effects with fresh ids as one undo step — the Effects &
  /// presets panel's Load preset action. A quiet error when there is no selected
  /// layer or the file cannot be read.
  Future<void> loadPresetOntoSelected() async {
    final compId = frontCompIdResolved;
    final layerId = selectedLayer;
    if (compId == null || layerId == null) {
      errorNotice = 'select a layer to load a preset onto';
      notifyListeners();
      return;
    }
    final path = await presetOpenPicker();
    if (path == null) return; // cancelled — leave the status line as-is
    final String text;
    try {
      text = await File(path).readAsString();
    } catch (e) {
      errorNotice = 'could not read the preset';
      notifyListeners();
      return;
    }
    loadEffectPreset(compId, layerId, text);
  }

  /// The realtime preview tier currently in force (Full/Half/Third/Quarter and
  /// its scale) — the Viewer readout and, in Auto mode, the next-frame scale.
  /// Falls back to Full with no bridge or an older library.
  BridgePlaybackTier playbackTier() => editOps?.playbackTier() ?? BridgePlaybackTier.full;

  /// Reset the realtime tier controller to Full (playback stopped, comp
  /// changed, or the resolution picker switched back to Auto).
  BridgePlaybackTier resetRealtime() =>
      editOps?.resetRealtime() ?? BridgePlaybackTier.full;

  // Recovery + boot log.

  /// List the rotating autosaves beside a project (empty [path] = the loaded
  /// one). Empty without a bridge or an older library.
  List<BridgeAutosave> listAutosaves(String path) =>
      editOps?.listAutosaves(path) ?? const [];

  /// Open a project and replay its crash journal on top (empty [path] = the
  /// loaded one) — the recovery modal's "restore journal" path.
  void restoreJournal(String path) {
    final e = editOps;
    if (e == null) return;
    _applyReply(e.restoreJournal(path), 'Recovered from journal');
  }

  /// Open a project from an explicit [path] — the recovery modal's "open last
  /// save" and "open an autosave" paths (mirrors [openProject] without the
  /// picker). When [rememberAs] is given the workspace remembers THAT path as
  /// the last project instead of [path]: opening an autosave keeps the real
  /// project as "last" so the next launch reopens the project, not the rotating
  /// copy (egui's `recover_from_autosave` keeps the project path the same way).
  ///
  /// Honest limit recorded on the ledger E row: the bridge has no
  /// load-content-but-keep-path op, so the engine's OWN loaded path follows
  /// [path]; a Save straight after opening an autosave therefore writes to the
  /// autosave copy until that op lands. Restore-journal and open-last-save are
  /// unaffected (both load the project's own path).
  void openPath(String path, {String? rememberAs}) {
    final b = bridge;
    if (b == null) {
      engine('Open project');
      return;
    }
    flushPendingSession();
    final reply = b.openProject(path);
    _applyReply(reply, 'Project opened');
    if (reply.ok) {
      _dirtySinceSave = false;
      final remember = rememberAs ?? path;
      _applySessionFor(remember);
      rememberProject?.call(remember);
    }
  }

  /// The engine's honest boot lines for the splash (empty without a bridge or an
  /// older library, so the splash keeps its canned lines then).
  List<String> bootLog() => editOps?.bootLog() ?? const [];

  // --- Section D additive state (editors, viewer tools, preview scale) ------
  //
  // Snapshot v5 carries a solid's colour (`layer.colour`) but not its size, and
  // does not carry text content or camera zoom back at all — only the setters
  // exist. These session maps remember what the user committed so the editors
  // read their own values back (annotated on the ledger: read-back awaits the
  // matching snapshot fields). All additive; no existing method changes.

  /// Text-layer content the user has committed this session, keyed by layer id.
  final Map<String, TextContent> textEdits = {};

  /// Solid-layer size the user has committed this session, keyed by layer id.
  final Map<String, SolidSize> solidSizeEdits = {};

  /// Camera-layer zoom the user has committed this session, keyed by layer id.
  final Map<String, double> cameraZoomEdits = {};

  /// The text content the editor shows for [layerId]: the snapshot read-back
  /// (`layer.text`, bridge v0.9) when it is present, else this session's edit,
  /// else the unedited default (empty, 72 pt, white).
  TextContent textContentFor(String layerId) {
    final snap = snapshot;
    if (snap != null) {
      final doc = _findLayer(snap, layerId)?.text;
      if (doc != null) {
        final fill = doc.fill.length >= 4
            ? [doc.fill[0], doc.fill[1], doc.fill[2], doc.fill[3]]
            : [
                doc.fill.isNotEmpty ? doc.fill[0] : 1.0,
                doc.fill.length > 1 ? doc.fill[1] : 1.0,
                doc.fill.length > 2 ? doc.fill[2] : 1.0,
                1.0,
              ];
        return TextContent(doc.content, doc.size, fill);
      }
    }
    return textEdits[layerId] ?? TextContent.initial;
  }

  /// The solid size the editor shows for [layerId]: the snapshot read-back
  /// (`layer.solidSize`, bridge v0.9) when present, else this session's edit,
  /// else a sensible default (the front comp's size, or 1920×1080).
  SolidSize solidSizeFor(String layerId) {
    final snap = snapshot;
    if (snap != null) {
      final size = _findLayer(snap, layerId)?.solidSize;
      if (size != null && size.length == 2) {
        return SolidSize(size[0], size[1]);
      }
    }
    final held = solidSizeEdits[layerId];
    if (held != null) return held;
    final comp = frontComp;
    return SolidSize(comp?.width ?? 1920, comp?.height ?? 1080);
  }

  /// The camera zoom the editor shows for [layerId]: the snapshot read-back
  /// (`layer.cameraZoom`, bridge v0.9) when present, else this session's edit,
  /// else a sensible default (the front comp width, a common AE default).
  double cameraZoomFor(String layerId) {
    final snap = snapshot;
    if (snap != null) {
      final zoom = _findLayer(snap, layerId)?.cameraZoom;
      if (zoom != null) return zoom.value;
    }
    return cameraZoomEdits[layerId] ?? (frontComp?.width ?? 1920).toDouble();
  }

  /// Commit a text layer's content (content, size, scene-linear fill), through
  /// `setTextContent`, remembering it so the editor reads it back.
  void commitTextContent(
      String compId, String layerId, TextContent content) {
    textEdits[layerId] = content;
    final c = content.rgba;
    setTextContent(compId, layerId, content.text, content.size, c[0], c[1],
        c[2], c.length > 3 ? c[3] : 1.0);
  }

  /// Commit a solid layer's colour and size, through `setSolid`, remembering the
  /// size so the editor reads it back (the colour reads back from the snapshot).
  void commitSolid(String compId, String layerId, List<double> rgba,
      SolidSize size) {
    solidSizeEdits[layerId] = size;
    setSolid(compId, layerId, rgba[0], rgba[1], rgba[2],
        rgba.length > 3 ? rgba[3] : 1.0, size.width, size.height);
  }

  /// Commit a camera layer's zoom, through `setCameraZoom`, remembering it so
  /// the editor reads it back.
  void commitCameraZoom(String compId, String layerId, double zoom) {
    cameraZoomEdits[layerId] = zoom;
    setCameraZoom(compId, layerId, zoom);
  }

  /// The selected layer's scene-linear solid colour from the snapshot (v3+),
  /// falling back to opaque mid-grey when absent.
  List<double> solidColourFor(BridgeLayer layer) {
    final c = layer.colour;
    if (c != null && c.length >= 3) {
      return [c[0], c[1], c[2], c.length > 3 ? c[3] : 1.0];
    }
    return const [0.5, 0.5, 0.5, 1.0];
  }

  // Viewer tool state (the toolbar) — the Dart mirror of the egui ToolMode /
  // ShapeKind, additive.

  ToolMode viewerTool = ToolMode.select;
  ShapeKind viewerShape = ShapeKind.rectangle;

  /// Select a Viewer tool (the toolbar buttons / the V·H·Q·G shortcuts).
  void setViewerTool(ToolMode mode) {
    if (viewerTool == mode) return;
    viewerTool = mode;
    notifyListeners();
  }

  /// Pick the Shape tool's mask shape (its right-click menu) and arm the Shape
  /// tool, exactly as the egui shape-picker context menu does.
  void setViewerShape(ShapeKind shape) {
    viewerShape = shape;
    viewerTool = ToolMode.shape;
    notifyListeners();
  }

  /// Draw the current shape as a mask on the selected layer from a Viewer
  /// drag rect `(x, y)` sized `w`×`h` in comp pixels (bridge v0.9
  /// `add_mask_geometry`, mirroring egui's Shape tool → real rect/ellipse/star
  /// geometry). A quiet notice when there is no selected layer (or front comp).
  void drawShapeMask(double x, double y, double w, double h) {
    final compId = frontCompIdResolved;
    final layerId = selectedLayer;
    if (compId == null || layerId == null) {
      errorNotice = 'select a layer to add a mask';
      notifyListeners();
      return;
    }
    addMaskGeometry(compId, layerId, viewerShape.opName, x, y, w, h);
  }

  // Preview render scale (the resolution picker in the transport).

  PreviewScale previewScale = PreviewScale.full;

  /// Auto resolution mode (the picker's "Auto" option, egui `preview_auto_res`):
  /// the preview renders at the realtime controller's live tier scale during
  /// playback, capped at Full. A manual pick clears it (overrides).
  bool previewAutoRes = false;

  /// The live realtime tier under Auto (bridge `playback_tier`): polled on the
  /// playhead cadence during playback, so the transport readout shows Full/Half/
  /// Third/Quarter as the engine adapts. Full when idle or on an older library.
  BridgePlaybackTier autoTier = BridgePlaybackTier.full;

  /// The scale the `PreviewSource` renders at: the live Auto tier's scale under
  /// Auto, else the manual picker's factor. Half/Third/Quarter downsample; Auto
  /// follows the realtime controller (K-171).
  double get effectivePreviewScale =>
      previewAutoRes ? autoTier.scale : previewScale.factor;

  /// Switch to Auto resolution: reset the realtime tier controller so the next
  /// playback re-measures from Full, and show Full until the first poll.
  void setPreviewAuto() {
    if (previewAutoRes) return;
    previewAutoRes = true;
    autoTier = resetRealtime();
    notifyListeners();
  }

  /// Set the preview render scale to a manual tier — this overrides Auto (the
  /// egui picker: any explicit Full/Half/Third/Quarter clears `preview_auto_res`).
  void setPreviewScale(PreviewScale scale) {
    if (!previewAutoRes && previewScale == scale) return;
    previewAutoRes = false;
    previewScale = scale;
    notifyListeners();
  }

  /// Poll the live realtime tier under Auto — the Viewer's playback ticker calls
  /// this on the playhead cadence WHILE PLAYING. Off Auto, or stopped, it holds
  /// the tier at Full so the readout reads "Auto" at rest. Fires the notifier
  /// only when the tier actually changes, so it never rebuilds per frame idly.
  void pollPlaybackTier() {
    if (!previewAutoRes) return;
    final next = playing ? playbackTier() : BridgePlaybackTier.full;
    if (next.tier != autoTier.tier) {
      autoTier = next;
      notifyListeners();
    }
  }

  // Eyedropper (the effect-controls colour dropper → Viewer sample).

  /// The armed eyedropper, or null when disarmed.
  EyedropperArm? eyedropperArm;

  /// Whether the eyedropper is armed (drives the Viewer magnifier overlay).
  bool get eyedropperArmed => eyedropperArm != null;

  /// Arm the eyedropper for a Colour effect parameter (its dropper button). The
  /// next Viewer click samples the shown frame and commits the colour.
  void armEyedropper(EyedropperArm arm) {
    eyedropperArm = arm;
    notifyListeners();
  }

  /// Disarm the eyedropper (a commit, Escape, or a click outside the image).
  void disarmEyedropper() {
    if (eyedropperArm == null) return;
    eyedropperArm = null;
    notifyListeners();
  }

  /// Commit a sampled scene-linear colour into the armed eyedropper's parameter
  /// (RGB from the pixel, alpha preserved), then disarm — the egui eyedropper
  /// commit path.
  void commitEyedropper(double r, double g, double b) {
    final arm = eyedropperArm;
    if (arm == null) return;
    setEffectParamColour(
        arm.compId, arm.layerId, arm.effectId, arm.paramName, r, g, b, arm.alpha);
    eyedropperArm = null;
    notifyListeners();
  }

  /// Render the front comp's current frame to CPU pixels for a one-off sample
  /// (the eyedropper's readback), or null when the composited-comp render is not
  /// available (no bridge, an older library, no GPU adapter). Full scale so the
  /// sampled pixel is the true colour, not a downsample.
  DecodedFrame? sampleCompFrame() {
    final b = bridge;
    final compId = frontCompIdResolved;
    if (b is! CompRenderBridge || compId == null) return null;
    return (b as CompRenderBridge).renderCompFrame(compId, previewFrame, 1.0);
  }

  // --- Bridge v0.8: rendered-frame cache + thumbnails ---------------------

  /// A hook the [PreviewSource] registers so "Clear cache" also empties the
  /// Dart-side decoded-frame LRU (the engine cache and the Dart image cache are
  /// two tiers of the same thing). Null when no Viewer is mounted.
  VoidCallback? previewCacheClearer;

  /// The rendered-frame cache controls, or null when there is no bridge or the
  /// loaded library predates ABI 8.
  CacheControlBridge? get cacheControl {
    final b = bridge;
    return b is CacheControlBridge && (b as CacheControlBridge).supportsCacheControl
        ? b as CacheControlBridge
        : null;
  }

  /// The thumbnail capability, or null when there is no bridge or the loaded
  /// library predates ABI 8. Exposed so the Project panel (another agent) can
  /// decode a footage row's thumbnail through one seam.
  ThumbnailBridge? get thumbnails {
    final b = bridge;
    return b is ThumbnailBridge && (b as ThumbnailBridge).supportsThumbnail
        ? b as ThumbnailBridge
        : null;
  }

  /// A cached thumbnail of footage [itemId] whose longer edge is at most
  /// [maxEdge], or null without the capability. Decoded and cached engine-side,
  /// so repeated calls are cheap.
  DecodedFrame? thumbnail(String itemId, int maxEdge) =>
      thumbnails?.thumbnail(itemId, maxEdge);

  /// The rendered-frame cache's live stats, or the empty default without the
  /// capability. The Timeline cache bar polls this on the app cadence (never
  /// per-paint) so it can size the RAM tier against the budget.
  BridgeCacheStats cacheStats() =>
      cacheControl?.cacheStats() ?? BridgeCacheStats.empty;

  // --- Cache bar warm-frame tracking (the RAM tier) -----------------------
  //
  // The bridge `cache_stats` export reports only aggregate counters
  // (used/budget/entries/hits/misses), not WHICH comp frames are warm — so the
  // Dart side records the frames it has itself driven into the engine cache to
  // draw egui's per-frame cache bar (previewing.rs `cache_bar`, the RAM tier).
  // The set is scoped to one (comp, scale): a document edit invalidates the
  // engine's rendered frames, so any adopt clears it, and changing the preview
  // scale re-scopes it (egui folds the quality tag into its bar memo key). The
  // [PreviewSource] calls [noteFrameWarmed] as each comp frame lands.

  final Set<int> _warmFrames = {};
  String? _warmComp;
  PreviewScale _warmScale = PreviewScale.full;

  /// Bumped whenever the warm-frame set changes, so the cache bar rebuilds off a
  /// fine-grained notifier rather than the big document one.
  final ValueNotifier<int> cacheBarRevision = ValueNotifier<int>(0);

  /// The warm comp frames (the RAM tier) for [compId] at the current preview
  /// scale, or an empty set when the scope has moved on.
  Set<int> warmFramesFor(String compId) =>
      _warmComp == compId && _warmScale == previewScale
          ? _warmFrames
          : const <int>{};

  /// Record that comp [compId] frame [frame] is now in the engine RAM cache
  /// (the [PreviewSource] calls this after a successful comp render). Re-scopes
  /// the set when the comp or the preview scale has changed.
  void noteFrameWarmed(String compId, int frame) {
    if (_warmComp != compId || _warmScale != previewScale) {
      _warmComp = compId;
      _warmScale = previewScale;
      _warmFrames.clear();
    }
    // Cap the tracked set to the engine's true entry count so a budget eviction
    // (LRU) is reflected honestly rather than overstating warmth. Approximate:
    // drop the oldest-noted frames when we exceed what the cache can hold.
    if (_warmFrames.add(frame)) {
      final entries = cacheStats().entries;
      if (entries > 0 && _warmFrames.length > entries) {
        final overflow = _warmFrames.length - entries;
        final oldest = _warmFrames.take(overflow).toList();
        _warmFrames.removeAll(oldest);
      }
      cacheBarRevision.value++;
    }
  }

  /// Forget every warm frame (Settings → Clear cache, and any document edit).
  void _invalidateWarmFrames() {
    if (_warmFrames.isEmpty) return;
    _warmFrames.clear();
    cacheBarRevision.value++;
  }

  /// Empty the rendered-frame cache now (Settings → Clear cache): the engine
  /// cache and the Dart-side decoded LRU together. A calm notice reports the
  /// result; on an older library it says the build is missing the control rather
  /// than silently doing nothing.
  void clearCache() {
    _invalidateWarmFrames();
    final c = cacheControl;
    if (c == null) {
      // Still empty the Dart-side tier; note the engine cache is unavailable.
      previewCacheClearer?.call();
      notice = 'cache cleared (this engine build has no frame cache)';
      notifyListeners();
      return;
    }
    final stats = c.clearCache();
    previewCacheClearer?.call();
    notice = 'cache cleared (${stats.entries} engine frames freed)';
    notifyListeners();
  }

  /// Set the rendered-frame cache's RAM budget in megabytes (Settings →
  /// Performance). A quiet no-op without the capability. Clamped to a sane
  /// minimum so a zero never disables caching by accident.
  void setCacheBudgetMb(int megabytes) {
    final c = cacheControl;
    if (c == null) return;
    final bytes = megabytes.clamp(16, 1 << 20) * 1024 * 1024;
    c.setCacheBudget(bytes);
  }

  // --- Bridge v0.4 export -------------------------------------------------

  /// Resolve a delivery [presetName] into the dialogue fields it stamps plus its
  /// suggested file name (the default fields without a bridge).
  BridgeExportPreset exportPreset(
          String presetName, String compName, String template) =>
      bridge?.exportPreset(presetName, compName, template) ??
      BridgeExportPreset.idle;

  /// Start an export of [compId] to [outPath] with the dialogue-shaped
  /// [specJson]. Returns the reply so the UI can queue on
  /// "an export is already running"; without a bridge it is a quiet no-op that
  /// reports failure. Does not refresh the snapshot (an export mutates nothing).
  BridgeReply startExport(String compId, String specJson, String outPath) {
    final b = bridge;
    if (b == null) {
      return const BridgeReply.err('no engine library');
    }
    final reply = b.startExport(compId, specJson, outPath);
    if (!reply.ok) errorNotice = reply.error;
    notifyListeners();
    return reply;
  }

  /// Poll the running export — the seam a UI timer drives (this state owns no
  /// timer of its own). Returns the idle state without a bridge.
  BridgeExportState pollExport() => bridge?.exportPoll() ?? BridgeExportState.idle;

  /// Ask the running export to cancel.
  void cancelExport() {
    bridge?.exportCancel();
    notifyListeners();
  }

  // --- Export queue + live progress (F4, export_actions.rs + app_update.rs) --
  //
  // A Dart-side one-at-a-time queue: confirming an export while one runs
  // enqueues it, and each completion (done/failed) starts the next. A shell
  // Timer drives [exportPollTick] at ~4 Hz while one runs; the status line
  // reads [exportStatusText]. Everything degrades to a quiet no-op without a
  // bridge (the menu/palette keep their F0 `engine` notices instead).

  final Queue<QueuedExport> _exportQueue = Queue<QueuedExport>();

  /// The running export's file name (the status-line label), or null when idle.
  String? exportName;

  /// The encoder the ladder settled on, once a running poll reports it — kept
  /// so the quiet completion notice can name it (the `done` poll carries no
  /// encoder, exactly as the egui `export_encoder` outlives the Progress
  /// events).
  String? exportEncoder;

  /// The running export's progress counters (0/0 until the first poll).
  int exportFrame = 0;
  int exportTotal = 0;

  /// Whether an export is in flight (drives the poll timer and the status line).
  bool get exportRunning => exportName != null;

  /// How many exports wait behind the running one.
  int get exportQueueLength => _exportQueue.length;

  /// The status-line export readout while one runs (null when idle) — the exact
  /// wording of app_update.rs: `exporting {name} {frame}/{total}`, with the
  /// encoder and queued-count suffixes.
  String? get exportStatusText {
    final name = exportName;
    if (name == null) return null;
    var line = 'exporting $name $exportFrame/$exportTotal';
    final enc = exportEncoder;
    if (enc != null) line += ' · $enc';
    if (_exportQueue.isNotEmpty) line += ' · ${_exportQueue.length} queued';
    return line;
  }

  /// Queue one export (the dialogue's confirm, or a share export). It starts
  /// immediately when nothing is running; otherwise it waits its turn
  /// (export_actions.rs `enqueue_export` + `try_start_next_export`).
  void queueExport(String compId, String specJson, String outPath) {
    _exportQueue.add(QueuedExport(compId, specJson, outPath, _fileName(outPath)));
    _tryStartNextExport();
  }

  /// Start the next queued export when none is running (a no-op otherwise).
  void _tryStartNextExport() {
    if (exportRunning || _exportQueue.isEmpty) return;
    final next = _exportQueue.first;
    // We only reach here idle, so a failure is a genuine start error (a bad
    // comp, no GPU) rather than "already running"; startExport has set the
    // error tint. Drop the item either way so the queue can never wedge.
    final reply = startExport(next.compId, next.specJson, next.outPath);
    _exportQueue.removeFirst();
    if (reply.ok) {
      exportName = next.name;
      exportEncoder = null;
      exportFrame = 0;
      exportTotal = 0;
      // Deliberately leave `errorNotice` alone: starting the next export must
      // not wipe the tint from an export that just failed (egui's
      // `try_start_next_export` never clears the error).
      notifyListeners();
    }
  }

  /// One poll tick — a shell Timer drives this at ~4 Hz while an export runs.
  /// Reads the bridge's export state, updates the status-line readout, and on a
  /// terminal state posts the quiet completion notice or the error tint with
  /// app_update.rs's exact wording, then starts the next queued export.
  void exportPollTick() {
    if (!exportRunning) return;
    final s = pollExport();
    switch (s.state) {
      case 'running':
        exportFrame = s.frame;
        exportTotal = s.total;
        if (s.encoder != null) exportEncoder = s.encoder;
        notifyListeners();
      case 'done':
        // A completed export is a quiet notice, not an error (docs/15 §10).
        final enc = exportEncoder;
        final withEnc = enc != null ? ' — encoded with $enc' : '';
        notice = 'exported ${s.path ?? ''}$withEnc';
        errorNotice = null;
        _clearExportSession();
        _tryStartNextExport();
        notifyListeners();
      case 'failed':
        errorNotice = 'export: ${s.error ?? 'failed'}';
        _clearExportSession();
        _tryStartNextExport();
        notifyListeners();
      default:
        // 'idle' while we believed one was running — treat as finished quietly.
        _clearExportSession();
        _tryStartNextExport();
        notifyListeners();
    }
  }

  void _clearExportSession() {
    exportName = null;
    exportEncoder = null;
    exportFrame = 0;
    exportTotal = 0;
  }

  /// The last path segment of [path] (its file name), for the status line.
  static String _fileName(String path) {
    final parts = path.split(RegExp(r'[/\\]'));
    for (final part in parts.reversed) {
      if (part.isNotEmpty) return part;
    }
    return 'export';
  }

  /// Start a size-targeted share export (K-037): resolve the front comp, size
  /// the video bitrate to [targetMb] via [shareExportBitRate], ask where to
  /// save, then queue it directly — no settings dialogue, exactly as
  /// `Shell::start_share_export` does. A quiet no-op without a bridge (the menu
  /// keeps its F0 notice for that build).
  Future<void> startShareExport(double targetMb) async {
    if (bridge == null) return;
    final comp = frontComp;
    final compId = frontCompIdResolved;
    if (comp == null || compId == null) {
      errorNotice = 'select a composition to export';
      notifyListeners();
      return;
    }
    final bitRate = shareExportBitRate(
      targetMb: targetMb,
      durationSeconds: _compDurationSeconds(comp),
      hasAudio: _compHasAudio(comp),
    );
    final suggested = 'share-${targetMb.toInt()}mb.mp4';
    final path = await exportSaveLocationPicker(suggested);
    if (path == null) return; // cancelled — leave the status line as-is
    // The bridge's spec resolver takes Mbps (blank = default quality); a share
    // export always pins an explicit bitrate. The comp's own size and the
    // leaner share AAC rate ride the spec too.
    final specJson = jsonEncode({
      'preset': 'custom',
      'codec': 'h264',
      'size': [comp.width, comp.height],
      'bitrate_mbps': (bitRate / 1000000.0).toString(),
      'include_audio': true,
      'audio_bit_rate': 192000,
    });
    queueExport(compId, specJson, path);
  }

  /// The export span in seconds — the work area when set, else the whole comp
  /// (a faithful mirror of `start_share_export`'s `duration`, before its
  /// 0.1 s floor, which [shareExportBitRate] applies).
  double _compDurationSeconds(BridgeComp comp) {
    final fps = comp.fps.fps;
    if (fps <= 0) return 0;
    final wa = comp.workArea;
    final frames =
        wa != null ? (wa[1] - wa[0]).toDouble() : comp.frameCount.toDouble();
    return frames / fps;
  }

  /// A best-effort read of whether [comp] carries audio: any audible footage
  /// layer whose source item probed with an audio track. The egui side asks the
  /// renderer for the comp's audio jobs; the snapshot cannot reproduce that
  /// exactly, so this approximates it (noted in the checklist).
  bool _compHasAudio(BridgeComp comp) {
    final snap = snapshot;
    if (snap == null) return false;
    for (final layer in comp.layers) {
      if (!layer.switches.audible) continue;
      final srcId = layer.sourceItemId;
      if (srcId == null) continue;
      if (_findItem(snap, srcId)?.media?.audio == true) return true;
    }
    return false;
  }

  /// Find a project item by its id across the snapshot tree, or null.
  BridgeItem? _findItem(BridgeSnapshot snap, String id) {
    BridgeItem? search(List<BridgeItem> items) {
      for (final item in items) {
        if (item.id == id) return item;
        final nested = search(item.children);
        if (nested != null) return nested;
      }
      return null;
    }

    return search(snap.items);
  }

  /// The front composition's display name (the `{comp}` filename token), or an
  /// empty string when there is no composition — for the export dialogue.
  String get frontCompName {
    final id = frontCompIdResolved;
    for (final c in compositions) {
      if (c.id == id) return c.name;
    }
    return '';
  }

  /// Run [op] against the bridge (a quiet no-op without one), applying its
  /// reply the same way [setLayerSwitch] and friends do.
  void _bridgeOp(BridgeReply Function(DocumentBridge b) op) {
    final b = bridge;
    if (b == null) return;
    _applyOp(op(b));
  }

  /// The current value of [layerId]'s transform [property]: the snapshot v3
  /// read-back when it is present, falling back to the session edit map (and
  /// null before any edit). The effect-controls panel adopts this so it shows
  /// true engine values once read-back lands, not only this session's edits.
  double? transformValueFor(String layerId, String property) {
    final snap = snapshot;
    if (snap != null) {
      final layer = _findLayer(snap, layerId);
      final prop = layer?.transform?[property];
      if (prop != null) return prop.value;
    }
    return transformEdits['$layerId/$property'];
  }

  /// A footage [layer]'s source media duration in seconds, from the probed
  /// project item (`media.durationFrames / media.fps`), or null when the source
  /// is unprobed / has no video / is not footage. The Timeline overrun HOLD
  /// hatch reads this alongside the layer's Retime store.
  double? sourceDurationSecsFor(BridgeLayer layer) {
    final id = layer.sourceItemId;
    final snap = snapshot;
    if (id == null || snap == null) return null;
    final media = _findItem(snap, id)?.media;
    if (media == null) return null;
    final fps = media.fps.fps;
    if (fps <= 0) return null;
    return media.durationFrames / fps;
  }

  /// Find a layer by its id across every composition in [snap], or null.
  BridgeLayer? _findLayer(BridgeSnapshot snap, String layerId) {
    BridgeLayer? search(List<BridgeItem> items) {
      for (final item in items) {
        final comp = item.comp;
        if (comp != null) {
          for (final l in comp.layers) {
            if (l.id == layerId) return l;
          }
        }
        final nested = search(item.children);
        if (nested != null) return nested;
      }
      return null;
    }

    return search(snap.items);
  }

  /// Decode one footage frame for the Viewer's CPU path, or null when there is
  /// no bridge or the frame cannot be decoded.
  DecodedFrame? decodeFrame(String itemId, int frame) =>
      bridge?.decodeFrame(itemId, frame);

  /// Apply a fine-grained op reply: refresh the snapshot on success (no chatty
  /// notice — these are direct manipulations, not menu actions), surface any
  /// failure in the error tint.
  void _applyOp(BridgeReply reply) {
    if (reply.ok) {
      _adoptSnapshot(reply.snapshot);
      errorNotice = null;
      // A successful direct edit dirties the document — the autosave gate (like
      // egui's `dirty`) so an idle session never writes rotating copies.
      _dirtySinceSave = true;
    } else {
      errorNotice = reply.error;
    }
    notifyListeners();
  }

  /// A monotonic document epoch, bumped whenever a fresh snapshot is adopted (a
  /// new engine document identity). The Project-panel thumbnails key their cache
  /// on it so a relink (or any edit) re-decodes rather than showing a stale
  /// picture; egui keys its own rendered-frame invalidation the same way.
  int documentEpoch = 0;

  /// Adopt a snapshot into the held state (undo/redo flags follow it). Keeps the
  /// playhead range in step with the front comp so the Timeline scrub and the
  /// End-key jump land on real frames.
  void _adoptSnapshot(BridgeSnapshot? snap) {
    if (snap == null) return;
    snapshot = snap;
    canUndo = snap.canUndo;
    canRedo = snap.canRedo;
    // A document edit invalidates the engine's rendered frames (and any
    // thumbnails a relink changed), so the cache bar's warm set resets and the
    // thumbnail epoch advances.
    documentEpoch++;
    _invalidateWarmFrames();
    final fc = frontComp;
    if (fc != null) {
      previewFrameCount = fc.frameCount;
      _setPlayhead(previewFrame.clamp(0, previewFrameCount));
    }
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

  // --- Per-project session (SavedSession parity) --------------------------
  //
  // The Flutter counterpart of the egui shell's `SavedSession`: which comps are
  // open, which is fronted, where the playhead sits, and which layer is
  // selected — persisted per project path and restored when it reopens. All
  // additive: without the [rememberSession]/[sessionFor] seams wired (the shell
  // points them at the Workspace), the app behaves exactly as before.

  /// The session as it stands right now, for the front project.
  SavedSession currentSession() => SavedSession(
        openComps: List<String>.from(openComps),
        activeComp: frontCompIdResolved,
        frame: previewFrame,
        selectedLayer: selectedLayer,
      );

  /// The trailing debounce for session writes (the perf pass): a continuous
  /// scrub or a burst of playhead/selection changes coalesces into ONE disk
  /// write ~500 ms after it settles, so no `Workspace.save()` fires per frame.
  static const Duration _sessionDebounce = Duration(milliseconds: 500);
  Timer? _sessionTimer;

  /// Schedule a debounced session write against the loaded project path, if one
  /// is known and the seam is wired. Repeated calls within [_sessionDebounce]
  /// collapse into a single trailing write — the fix for per-frame persistence.
  void _scheduleSessionPersist() {
    if (snapshot?.path == null || rememberSession == null) return;
    _sessionTimer?.cancel();
    _sessionTimer = Timer(_sessionDebounce, flushPendingSession);
  }

  /// Write the pending session now, cancelling any scheduled write. Called on
  /// dispose and on project close (open/new), and by tests that assert
  /// persistence without waiting out the debounce.
  @visibleForTesting
  void flushPendingSession() {
    _sessionTimer?.cancel();
    _sessionTimer = null;
    final path = snapshot?.path;
    if (path == null) return;
    rememberSession?.call(path, currentSession());
  }

  /// Apply the stored session for [path] after its project opens: front the
  /// saved comp, restore the playhead and the selection — each validated
  /// against the freshly-loaded document so a stale id falls back to the
  /// default rather than crashing.
  void _applySessionFor(String path) {
    final read = sessionFor;
    if (read == null) return;
    final session = read(path);
    if (session == null) return;
    final comps = compositions;
    // Restore the open-comp list to the ids that still exist.
    openComps
      ..clear()
      ..addAll([
        for (final id in session.openComps)
          if (comps.any((c) => c.id == id)) id,
      ]);
    // Front the saved comp when it still resolves.
    final active = session.activeComp;
    if (active != null && comps.any((c) => c.id == active)) {
      frontCompId = active;
      previewFrameCount = frontComp?.frameCount ?? previewFrameCount;
    }
    // Restore the playhead, clamped into the (possibly changed) range.
    _setPlayhead(session.frame.clamp(0, previewFrameCount));
    // Restore the selection only if that layer is still present.
    final sel = session.selectedLayer;
    selectedLayer =
        (sel != null && snapshot != null && _findLayer(snapshot!, sel) != null)
            ? sel
            : null;
  }

  // --- Autosave (lumit_project::autosave parity) --------------------------
  //
  // A periodic rotating copy beside the project (`autosaves/<stem>.autosave-N.
  // lum`), written only when the document is dirty and has a path — the main
  // file is never touched. The timer is opt-in ([startAutosave]); tests drive
  // [autosaveTick] with an injected clock instead.

  bool _dirtySinceSave = false;
  DateTime _lastAutosave = DateTime.now();
  Timer? _autosaveTimer;

  /// Whether an autosave would write now (dirty, a bridge, and a saved path).
  bool get autosaveEligible =>
      _dirtySinceSave && bridge != null && snapshot?.path != null;

  /// Start the periodic autosave driver: every [checkEvery] it asks
  /// [autosaveTick] whether a write is due. Idempotent — a running timer is
  /// replaced. The shell calls this once a bridge is live; tests need not.
  void startAutosave({Duration checkEvery = const Duration(seconds: 30)}) {
    _autosaveTimer?.cancel();
    _lastAutosave = DateTime.now();
    _autosaveTimer =
        Timer.periodic(checkEvery, (_) => autosaveTick(DateTime.now()));
  }

  /// Stop the periodic autosave driver (no-op when none runs).
  void stopAutosave() {
    _autosaveTimer?.cancel();
    _autosaveTimer = null;
  }

  /// One autosave check at [now]: writes a rotating copy when the interval has
  /// elapsed and the document is dirty. The seam a timer (or a test clock)
  /// drives. Returns true when a copy was written.
  bool autosaveTick(DateTime now) {
    if (!autosaveEligible) return false;
    if (now.difference(_lastAutosave) < autosaveInterval) return false;
    _lastAutosave = now;
    return _writeAutosave();
  }

  /// Write one rotating autosave copy now, regardless of the interval (used by
  /// [autosaveTick] and available to a manual "save a copy" path).
  ///
  /// The dedicated `lumit_bridge_autosave` op (v0.5) writes a rotating copy
  /// beside the project WITHOUT re-pointing the engine's loaded path — closing
  /// the known drift gap where the old `saveProject`-based autosave silently
  /// pointed Save at the autosave file. When the loaded library carries the
  /// capability we route through it (it does its own rotation); an older
  /// library falls back to the previous rotate-then-`saveProject` path. Either
  /// way the reply is NOT adopted, so the held snapshot keeps the real path.
  bool _writeAutosave() {
    final b = bridge;
    final path = snapshot?.path;
    if (b == null || path == null) return false;
    final e = editOps;
    final BridgeReply reply;
    if (e != null) {
      // The dedicated op rotates and writes without repointing the path.
      reply = e.autosave('', autosaveKeep);
    } else {
      // Older library: rotate the folder ourselves, then write via saveProject.
      final slot1 = AutosaveScheme.rotateAndNewestSlot(path, autosaveKeep);
      reply = b.saveProject(slot1);
    }
    if (reply.ok) {
      // Autosave is silent in the egui frontend (no status-line notice), so
      // nothing is surfaced here beyond clearing the dirty gate.
      _dirtySinceSave = false;
      return true;
    }
    errorNotice = reply.error;
    notifyListeners();
    return false;
  }

  @override
  void dispose() {
    // Flush any pending session write, then tear down the timers/notifiers.
    flushPendingSession();
    _autosaveTimer?.cancel();
    _previewSource?.dispose();
    playheadFrame.dispose();
    cacheBarRevision.dispose();
    super.dispose();
  }
}
