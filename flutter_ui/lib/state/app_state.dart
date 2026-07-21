// The Phase-F0 stand-in for the engine-backed application state. In the egui
// frontend this is `AppState` (crates/lumit-ui/src/app_state/), owned by Rust;
// here it is a small ChangeNotifier that answers the chrome's questions and
// records the actions the chrome dispatches, so every menu item, shortcut and
// panel control can be wired now and re-pointed at the bridge in Phase F1
// (docs/flutter-port/03-ARCHITECTURE.md).

import 'dart:io';

import 'package:flutter/foundation.dart';

import '../bridge/bridge.dart';
import '../panels/preview_source.dart';
import 'file_dialogs.dart';

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

  /// The selected layer, by its snapshot layer id (was an int index in F0; the
  /// Timeline selects by the engine's stable layer id so ops address the right
  /// layer). Null when nothing is selected.
  String? selectedLayer;

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

  /// Move the playhead during playback WITHOUT stopping (the Viewer's transport
  /// ticker drives this). Unlike [goToFrame] it leaves `playing` set, so the
  /// loop keeps running. Additive F2 seam.
  void advancePlayback(int frame) {
    previewFrame = frame;
    notifyListeners();
  }

  /// The Viewer's CPU frame source (phase F2), shared with the Scopes panel so
  /// both read the same decoded pixels. Created lazily on first use; harmless
  /// without a bridge (it simply never resolves a frame). Single-layer preview
  /// until the compositor is extracted from the egui crate.
  PreviewSource? _previewSource;
  PreviewSource get previewSource => _previewSource ??= PreviewSource(this);

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
    previewFrame = previewFrame.clamp(0, previewFrameCount);
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
    } else {
      errorNotice = reply.error;
    }
    notifyListeners();
  }

  /// Adopt a snapshot into the held state (undo/redo flags follow it). Keeps the
  /// playhead range in step with the front comp so the Timeline scrub and the
  /// End-key jump land on real frames.
  void _adoptSnapshot(BridgeSnapshot? snap) {
    if (snap == null) return;
    snapshot = snap;
    canUndo = snap.canUndo;
    canRedo = snap.canRedo;
    final fc = frontComp;
    if (fc != null) {
      previewFrameCount = fc.frameCount;
      previewFrame = previewFrame.clamp(0, previewFrameCount);
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
}
