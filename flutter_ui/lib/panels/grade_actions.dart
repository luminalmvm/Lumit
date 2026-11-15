// The grading commands that more than one place runs: the Layer menu's Grade
// rows, the Colour keys, the Grade nodes panel and the Shots panel.
//
// Each one works on the current shot (the picked Cut clip, else the selected
// layer) and its current node, or on a shot it is handed, and each edit is
// one undo step.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/grade_cdl.dart';
import 'package:lumit_flutter/src/rust/api/grade_groups.dart';
import 'package:lumit_flutter/src/rust/api/grade_lut.dart';
import 'package:lumit_flutter/src/rust/api/grade_shared.dart';
import 'package:lumit_flutter/src/rust/api/grade_shots.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/presets.dart';
import 'package:lumit_flutter/src/rust/api/stills.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../shell/dialog_frame.dart' show askLumitChoice;
import '../shell/settings_account.dart' show askBefore;
import '../shell/theme_name_dialog.dart';
import '../state/dock.dart';
import '../state/file_dialogs.dart';
import '../state/grade_engine.dart';
import '../state/grade_target.dart';
import '../state/timecode.dart';
import 'effect_param_row_frb.dart'
    show cachedListParameters, defaultEffectValue;
import 'grade_clipboard.dart';
import 'grade_memories.dart';
import 'layer_fold_frb.dart' show asOneUndoStep;
import 'viewer_grade.dart' show gradeNodeOf;

const GradeEngine _engine = GradeEngine();

/// The rows a reset leaves alone: what a node is, not what it does. A group
/// grade's part says which of the group's grades it stands for.
const Set<String> _structure = {'space', 'combine', 'part'};

void _tell(LumitState app, LumitUiState ui) {
  app.notifyDocumentChanged();
  ui.model.refresh();
}

/// The live handles of [shot]'s stack, for an edit.
List<BridgeEffectInstance> shotHandles(GradeShot shot) => shot.clip == null
    ? shot.layer.getEffects()
    : shot.layer.getClipEffects(clip: shot.clip!, onCompClock: true);

/// The layer's name, or for a clip the name of what it plays.
String gradeShotName(LumitUiState ui, GradeShot shot) {
  final info = ui.model.byId(shot.layer.internallayerId)?.info;
  if (info == null) return '';
  for (final c in info.clips) {
    if (c.id == shot.clip) return c.sourceName;
  }
  return info.name;
}

/// A name a file can take.
String fileSafe(String name) => name.replaceAll(RegExp(r'[/\\:*?"<>|]'), '-');

// --- Group grades ------------------------------------------------------------

/// [shot]'s group, read once per document revision.
(int?, String, UuidValue?)? _groupRead;

/// The group grade the panels show on the current shot while the scope is a
/// group's: the group and which of its grades. Null for the shot's own, and
/// for a shot that is in no group.
({UuidValue group, BridgeGradePart part})? gradeScopeOf(LumitUiState ui) {
  final shot = ui.gradeShot.value;
  final part = switch (ui.grade.scope.value) {
    GradeScope.groupPre => BridgeGradePart.pre,
    GradeScope.groupPost => BridgeGradePart.post,
    _ => null,
  };
  if (shot == null || part == null) return null;
  final revision = ui.model.revision;
  final held = _groupRead;
  UuidValue? group;
  if (held != null && held.$1 == revision && held.$2 == shot.key) {
    group = held.$3;
  } else {
    try {
      group = shot.layer.getGradeGroup(clip: shot.clip);
    } catch (_) {
      group = null;
    }
    _groupRead = (revision, shot.key, group);
  }
  return group == null ? null : (group: group, part: part);
}

// --- Shared nodes ------------------------------------------------------------

/// Every shared node, read once per document revision.
(int?, List<BridgeSharedNode>)? _sharedRead;

List<BridgeSharedNode> sharedGradeNodes(LumitUiState ui) {
  final revision = ui.model.revision;
  final held = _sharedRead;
  if (held != null && held.$1 == revision) return held.$2;
  List<BridgeSharedNode> read;
  try {
    read = ui.grade.project?.getSharedGradeNodes() ?? const [];
  } catch (_) {
    read = const [];
  }
  _sharedRead = (revision, read);
  return read;
}

/// The shared node a reference in a shot's stack stands for, if it is one.
BridgeSharedNode? sharedNodeOf(LumitUiState ui, UuidValue? reference) =>
    reference == null
        ? null
        : sharedGradeNodes(ui)
            .where((n) => n.references.contains(reference))
            .firstOrNull;

/// The list the current node's rows live in on the current shot when it is
/// not the shot's own stack: the group grade in view, or the shared node
/// picked in the node strip, as its one-node list.
({UuidValue group, BridgeGradePart part})? gradeListOf(LumitUiState ui) {
  final scope = gradeScopeOf(ui);
  if (scope != null) return scope;
  final shared = sharedNodeOf(ui, ui.gradeNode.value);
  return shared == null ? null : (group: shared.id, part: BridgeGradePart.pre);
}

// --- Nodes -------------------------------------------------------------------

/// The node the Grade panel edits on [shot]: the one picked there while it is
/// still in the stack, else the last Correction.
BridgeEffectInstanceInfo? currentGradeNode(LumitUiState ui,
    [GradeShot? shot]) {
  final s = shot ?? ui.gradeShot.value;
  if (s == null) return null;
  final stack = shotStackOf(ui, s) ?? const [];
  final want = s.same(ui.gradeShot.value) ? ui.gradeNode.value : null;
  return stack.where((e) => e.id == want).firstOrNull ??
      stack.where((e) => e.name == correctionName).lastOrNull;
}

/// Whether a [kind] node can go on the current shot now. An outside node
/// needs a Correction to take the key of.
bool canAddGradeNode(LumitUiState ui, GradeNodeKind kind) {
  final shot = ui.gradeShot.value;
  if (shot == null || shotStackOf(ui, shot) == null) return false;
  if (!_engine.canAdd(kind, shot)) return false;
  return kind != GradeNodeKind.outside ||
      currentGradeNode(ui)?.name == correctionName;
}

/// A new node after [after] (the current node when null, else the last grade
/// node), made the current one.
UuidValue? addGradeNode(LumitState app, LumitUiState ui, GradeNodeKind kind,
    {UuidValue? after}) {
  final shot = ui.gradeShot.value;
  if (shot == null || !canAddGradeNode(ui, kind)) return null;
  final behind = after ??
      currentGradeNode(ui)?.id ??
      gradeNodesOf(shotStackOf(ui, shot) ?? const []).lastOrNull?.id;
  UuidValue? made;
  asOneUndoStep(app.project, () {
    made = _engine.addNode(shot, behind, kind);
  }, name: 'Add node');
  if (made != null) ui.grade.setNode(made);
  _tell(app, ui);
  return made;
}

/// Whether a node of another Colour effect can go on the current shot now.
bool canAddGradeEffect(LumitUiState ui) {
  final shot = ui.gradeShot.value;
  return shot != null && shotStackOf(ui, shot) != null;
}

/// Effect [name] as a new node after [after] (the current node when null,
/// else the shot's last grade node), made the current one. A group's grade
/// in view takes it at its end. One step.
UuidValue? addGradeEffectNode(LumitState app, LumitUiState ui, String name,
    {UuidValue? after}) {
  final shot = ui.gradeShot.value;
  if (shot == null || !canAddGradeEffect(ui)) return null;
  final scope = gradeScopeOf(ui);
  final behind = after ??
      currentGradeNode(ui)?.id ??
      gradeNodesOf(shotStackOf(ui, shot) ?? const []).lastOrNull?.id;
  UuidValue? made;
  asOneUndoStep(app.project, () {
    try {
      if (scope != null) {
        shot.layer.addGradeGroupEffect(
            group: scope.group, part_: scope.part, name: name);
        made = shot.layer
            .getGradeGroupEffects(group: scope.group, part_: scope.part)
            .lastOrNull
            ?.id();
        return;
      }
      final before = {for (final h in shotHandles(shot)) h.id()};
      shot.clip == null
          ? shot.layer.addEffect(name: name)
          : shot.layer.addClipEffect(clip: shot.clip!, name: name);
      final handles = shotHandles(shot);
      final h = handles.where((h) => !before.contains(h.id())).firstOrNull;
      if (h == null) return;
      final at = behind == null
          ? 0
          : handles.indexWhere((x) => x.id() == behind) + 1;
      shot.layer.reorderEffect(effect: h, newIndex: at);
      made = h.id();
    } catch (_) {
      // A locked layer refuses; the document is as it was.
    }
  }, name: 'Add node');
  if (made != null) ui.grade.setNode(made);
  _tell(app, ui);
  return made;
}

/// Every grade node on every picked shot is off.
bool gradeBypassed(LumitUiState ui) {
  final shot = ui.gradeShot.value;
  if (shot == null) return false;
  final nodes = gradeNodesOf(shotStackOf(ui, shot) ?? const []);
  return nodes.isNotEmpty && nodes.every((e) => !e.enabled);
}

/// Whether the current shot has a grade to act on.
bool hasGrade(LumitUiState ui) {
  final shot = ui.gradeShot.value;
  return shot != null && shotGradeIds(ui, shot).isNotEmpty;
}

/// Switch the grade off on every picked shot, or on again when it is off.
void bypassGrade(LumitState app, LumitUiState ui) {
  final off = !gradeBypassed(ui);
  final targets = gradeTargets(ui);
  if (targets.isEmpty) return;
  asOneUndoStep(app.project, () {
    for (final s in targets) {
      final ids = shotGradeIds(ui, s).toSet();
      try {
        for (final h in shotHandles(s)) {
          if (ids.contains(h.id())) {
            s.layer.setEffectEnabled(effect: h, enabled: !off);
          }
        }
      } catch (_) {
        // A locked layer refuses, and the rest still take it.
      }
    }
  }, name: 'Bypass grade');
  _tell(app, ui);
}

/// Every row of [ids] on [shot] back to its default, keys and all, as
/// Resolve's reset does. What a node is (its space, how it combines) stays.
void resetNodesOn(GradeShot shot, Set<UuidValue> ids) {
  final handles = shotHandles(shot);
  resetNodesOnHandles(handles, ids);
  shot.layer.setEffects(effects: handles, clip: shot.clip);
}

/// [resetNodesOn] on staged [handles], for the caller to commit.
void resetNodesOnHandles(
    List<BridgeEffectInstance> handles, Set<UuidValue> ids) {
  for (final h in handles) {
    if (!ids.contains(h.id())) continue;
    for (final p in cachedListParameters(h.getInfo().name)) {
      if (_structure.contains(p.id)) continue;
      if (defaultEffectValue(p.kind) case final d?) {
        h.setValue(id: p.id, value: d);
      }
    }
  }
}

/// Every grade node on every picked shot, or on [onto], back to its
/// defaults.
void resetGrade(LumitState app, LumitUiState ui, {List<GradeShot>? onto}) {
  final targets = onto ?? gradeTargets(ui);
  if (targets.isEmpty) return;
  asOneUndoStep(app.project, () {
    for (final s in targets) {
      try {
        resetNodesOn(s, shotGradeIds(ui, s).toSet());
      } catch (_) {
        // A locked layer refuses, and the rest still take it.
      }
    }
  }, name: 'Reset grade');
  _tell(app, ui);
}

// --- The Viewer --------------------------------------------------------------

/// Off, Highlight, Key and round again, on the current node.
void cycleMatteView(LumitUiState ui) {
  final shot = ui.gradeShot.value;
  final comp = ui.selectedComp;
  final next = GradeMatte
      .values[(ui.grade.matte.value.index + 1) % GradeMatte.values.length];
  ui.grade.matte.value = next;
  if (shot == null || comp == null) return;
  _engine.setMatteView(comp, shot, gradeNodeOf(ui, shot)?.id, next,
      view: ui.activeViewId, backdrop: ui.grade.matteBackdrop.value);
  ui.requestFrame();
}

/// Show or hide the reference beside the picture. False with none set.
bool toggleGradeCompare(LumitUiState ui) {
  if (ui.grade.reference.value == null) return false;
  ui.grade.comparing.value = !ui.grade.comparing.value;
  return true;
}

/// The Lightbox: the Shots panel filling the window as a grid of large
/// pictures, or back as it was. Escape leaves it as it leaves any pane that
/// fills the window.
bool toggleLightbox(LumitUiState ui) {
  final on = !ui.grade.lightbox.value;
  final pane = Panel.shots.pane();
  if (on) {
    setPanelVisible(ui.split, Panel.shots, true);
    activatePanelTab(ui.split, Panel.shots);
    ui.activePane.value = pane;
    ui.maximisedPane.value = pane;
    ui.workspace.touch();
  } else if (ui.maximisedPane.value == pane) {
    ui.maximisedPane.value = null;
  }
  ui.grade.lightbox.value = on;
  return true;
}

/// The Viewer's split-screen grid of [what], or off when it already shows
/// that.
void toggleGradeGrid(LumitUiState ui, GradeGrid what) {
  final g = ui.grade.grid;
  g.value = g.value == what ? null : what;
  ui.requestFrame();
}

// --- Stills, looks and LUTs --------------------------------------------------

bool canGrabStill(LumitUiState ui) =>
    ui.selectedComp != null && ui.gradeShot.value != null;

/// Bumped whenever a still is made, so an open Gallery lists it.
final ValueNotifier<int> galleryStillsChanged = ValueNotifier(0);

/// What a new still is called: its shot and the time it was taken, as
/// `faces-warm.mp4 1:16`, the Viewer's clock without its leading zeros.
String stillName(LumitUiState ui, GradeShot shot, int frame) {
  final (num, den) = ui.model.fpsExact;
  final clock = timecodeOfRate(frame + ui.model.startFrame, num, den);
  return '${gradeShotName(ui, shot)} ${shortClock(clock)}'.trim();
}

/// A clock face without its leading zeros: 00:00:01:16 reads 1:16.
String shortClock(String clock) {
  var short = clock;
  while (short.startsWith('00:') && short.length > 5) {
    short = short.substring(3);
  }
  return short.startsWith('0') && short.length > 4 ? short.substring(1) : short;
}

/// Keep the frame at the playhead, with the current shot's grade, in the
/// Gallery's open album. With [of], that shot's grade at the playhead, or
/// at its middle frame when the playhead is outside it. Answers the still,
/// which an open Gallery lists.
Future<BridgeStill?> grabStill(LumitState app, LumitUiState ui,
    {BridgeShot? of}) async {
  final shot = of == null ? ui.gradeShot.value : shotOf(of);
  final comp = ui.selectedComp;
  if (shot == null || comp == null) return null;
  var frame = ui.playheadFrame.value;
  if (of != null && (frame < of.startFrame || frame >= of.endFrame)) {
    frame = (of.startFrame + of.endFrame) ~/ 2;
  }
  final BridgeStill still;
  try {
    var album = ui.grade.album.value;
    album ??= (await listStillAlbums()).firstOrNull;
    album ??= await makeStillAlbum(name: l10n.gradeStillsAlbum);
    still = await comp.grabStill(
        frame: frame,
        layer: shot.layer.internallayerId,
        clip: shot.clip,
        album: album,
        name: stillName(ui, shot, frame),
        colourView: ui.colourView);
    ui.grade.album.value = album;
  } catch (_) {
    app.postNotice(l10n.gradeStillFailed, error: true);
    return null;
  }
  galleryStillsChanged.value++;
  app.postNotice(l10n.gradeStillGrabbed);
  return still;
}

/// Write [shot]'s grade as a preset in the Looks group, made on the first
/// save. Answers the name it was saved as.
Future<String?> saveShotLook(BuildContext context, LumitState app,
    LumitUiState ui, GradeShot shot) async {
  final ids = shotGradeIds(ui, shot);
  if (ids.isEmpty) return null;
  // The shot's name to start from, less a file's extension.
  final name = await askThemeName(context,
      title: l10n.gradeSaveLookTitle,
      suggested: gradeShotName(ui, shot)
          .replaceFirst(RegExp(r'\.[A-Za-z0-9]{2,4}$'), ''),
      confirm: l10n.save,
      label: l10n.name);
  final library = presetsDirPath();
  if (name == null || library == null || !context.mounted) return null;
  final sep = Platform.pathSeparator;
  final file = File('$library$sep$looksGroup$sep${fileSafe(name)}.lumfx');
  if (file.existsSync() &&
      !await askBefore(
          context, l10n.gradeLookReplace(name), l10n.gradeLookReplaceYes)) {
    return null;
  }
  try {
    makePresetGroup(group: looksGroup);
    // A clip's grade as the copied text, which is the same document a
    // preset is; a layer's through the preset writer.
    file.writeAsStringSync(shot.clip == null
        ? shot.layer.savePresetParts(
            name: name,
            effects: ids,
            transformKeys: false,
            textAnimators: false)
        : shot.layer.copyEffects(effects: ids));
  } catch (_) {
    // A folder that will not take the file. The document is untouched.
    return null;
  }
  ui.presetsChanged.value++;
  app.postNotice(l10n.gradeLookSavedNotice);
  return name;
}

/// Ask where, then write [shot]'s grade (the current shot's when null) as a
/// `.cube`.
Future<void> exportShotLut(LumitState app, LumitUiState ui,
    {GradeShot? shot,
    int size = 33,
    BridgeLutSpace space = BridgeLutSpace.video}) async {
  final s = shot ?? ui.gradeShot.value;
  final comp = ui.selectedComp;
  if (s == null || comp == null) return;
  final path = await pickLutSaveLocation(fileSafe(gradeShotName(ui, s)));
  if (path == null) return;
  try {
    await comp.exportGradeLut(
      layer: s.layer.internallayerId,
      clip: s.clip,
      frame: ui.playheadFrame.value,
      path: path,
      size: size,
      space: space,
    );
  } catch (_) {
    app.postNotice(l10n.gradeLutFailed, error: true);
    return;
  }
  app.postNotice(l10n.gradeLutExported);
}

// --- CDL ----------------------------------------------------------------------

/// Ask for a `.cdl`, `.cc` or `.ccc` and lay it on the current shot as a CDL
/// node after [after] (the current node, else the shot's last grade node),
/// made current. A collection asks which of its corrections. One step.
Future<void> importCdlFile(
    BuildContext context, LumitState app, LumitUiState ui,
    {UuidValue? after}) async {
  final shot = ui.gradeShot.value;
  if (shot == null) return;
  final path = await pickCdlToOpen();
  if (path == null || !context.mounted) return;
  List<String> ids;
  try {
    ids = cdlIds(path: path);
  } catch (_) {
    app.postNotice(l10n.gradeCdlFailed, error: true);
    return;
  }
  String? id;
  if (ids.length > 1) {
    id = await askLumitChoice<String>(
        context: context,
        title: l10n.gradeCdlPick,
        stacked: true,
        answers: [for (final i in ids) (i, i)]);
    if (id == null) return;
  }
  // Only a node of the shot's own stack can be followed: a group's grade in
  // view hands it after the shot's current node instead.
  final own = shotStackOf(ui, shot) ?? const [];
  final behind = own.any((e) => e.id == after)
      ? after
      : currentGradeNode(ui)?.id;
  try {
    final made = await shot.layer
        .importCdl(clip: shot.clip, after: behind, path: path, id: id);
    ui.grade.setNode(made);
  } catch (_) {
    app.postNotice(l10n.gradeCdlFailed, error: true);
    return;
  }
  _tell(app, ui);
}

/// Whether node [e] can be written as a CDL: a Correction or a CDL node.
bool cdlExportable(String effectName) =>
    effectName == correctionName || effectName == 'cdl';

/// Ask where, then write node [node] of the current shot as a CDL. A
/// Correction that needs more than a CDL holds is fitted, and the notice says
/// so.
Future<void> exportCdlFile(LumitState app, LumitUiState ui, UuidValue node,
    {String? name}) async {
  final shot = ui.gradeShot.value;
  final comp = ui.selectedComp;
  if (shot == null || comp == null) return;
  final path =
      await pickCdlSaveLocation(fileSafe(name ?? gradeShotName(ui, shot)));
  if (path == null) return;
  bool exact;
  try {
    exact = await comp.exportCdl(
        layer: shot.layer.internallayerId,
        clip: shot.clip,
        frame: ui.playheadFrame.value,
        node: node,
        path: path);
  } catch (_) {
    app.postNotice(l10n.gradeCdlExportFailed, error: true);
    return;
  }
  app.postNotice(exact ? l10n.gradeCdlExported : l10n.gradeCdlFitted);
}

/// A still's grade in place of the grade on [onto].
Future<void> applyStillGrade(LumitState app, LumitUiState ui, String path,
    List<GradeShot> onto) async {
  if (onto.isEmpty) return;
  final String text;
  try {
    text = await readStillGrade(path: path);
  } catch (_) {
    return;
  }
  layGrade(app, ui, text, name: 'Apply grade', to: onto);
}

// --- Solvers -----------------------------------------------------------------

/// [rows] written into [node] on [shot] as one step with what came before
/// it in the open undo group.
void _writeRows(GradeShot shot, UuidValue node, GradeRows rows) {
  final handles = shotHandles(shot);
  for (final h in handles) {
    if (h.id() != node) continue;
    for (final r in rows.entries) {
      h.setValue(id: r.key, value: r.value);
    }
  }
  shot.layer.setEffects(effects: handles, clip: shot.clip);
}

/// Run [solve] on [shot]'s current node, a new Correction when it has none,
/// and write what it answers, all as one step.
Future<void> _solveOn(
    LumitState app,
    LumitUiState ui,
    GradeShot shot,
    String name,
    Future<GradeRows?> Function(UuidValue node) solve) async {
  final project = app.project;
  if (project == null) return;
  project.beginUndoGroup();
  project.nameUndoGroup(name: name);
  var wrote = false;
  try {
    var node = currentGradeNode(ui, shot);
    if (node?.name != correctionName) node = null;
    final id = node?.id ??
        _engine.addNode(
            shot,
            gradeNodesOf(shotStackOf(ui, shot) ?? const []).lastOrNull?.id,
            GradeNodeKind.serial);
    if (id == null) return;
    wrote = node == null;
    final rows = await solve(id);
    if (rows != null && rows.isNotEmpty) {
      _writeRows(shot, id, rows);
      wrote = true;
    }
  } catch (_) {
    // The shot went, or the solver had nothing to read.
  } finally {
    project.endUndoGroup();
  }
  if (wrote) _tell(app, ui);
}

/// Neutralise the current shot's cast and set its black and white points.
Future<void> autoBalanceShot(LumitState app, LumitUiState ui) async {
  final shot = ui.gradeShot.value;
  final comp = ui.selectedComp;
  if (shot == null || comp == null) return;
  final frame = ui.playheadFrame.value;
  await _solveOn(app, ui, shot, 'Auto balance',
      (node) => _engine.autoBalance(comp, shot, frame, node));
}

/// Bring [shot] to look like [to] at comp frame [toFrame].
Future<void> matchShotTo(LumitState app, LumitUiState ui, BridgeShot shot,
    GradeShot to, int toFrame) async {
  final comp = ui.selectedComp;
  if (comp == null) return;
  final s = GradeShot(shot.layer, shot.clip);
  final frame = (shot.startFrame + shot.endFrame) ~/ 2;
  final reference = GradeReference.shot(to,
      frame: toFrame, name: gradeShotName(ui, to));
  await _solveOn(app, ui, s, 'Match shot',
      (node) => _engine.matchShot(comp, s, frame, node, reference));
}

// --- Shots -------------------------------------------------------------------

/// The comp's shots, read once per document revision.
(Object?, int?, List<BridgeShot>)? _shotsRead;

List<BridgeShot> compShots(LumitUiState ui) {
  final comp = ui.selectedComp;
  if (comp == null) return const [];
  final revision = ui.model.revision;
  final held = _shotsRead;
  if (held != null && held.$1 == comp.internalid && held.$2 == revision) {
    return held.$3;
  }
  List<BridgeShot> read;
  try {
    read = comp.shots();
  } catch (_) {
    read = const [];
  }
  _shotsRead = (comp.internalid, revision, read);
  return read;
}

GradeShot shotOf(BridgeShot s) => GradeShot(s.layer, s.clip);

/// Where the current shot is in [shots], or -1.
int currentShotIndex(LumitUiState ui, List<BridgeShot> shots) {
  final current = ui.gradeShot.value;
  if (current == null) return -1;
  return shots.indexWhere((s) => shotOf(s).same(current));
}

/// Make [current] the current shot with [picked] picked beside it: the
/// layer selection, and the Cut pick for clips. The playhead goes into the
/// shot when it is outside it.
void pickShots(LumitUiState ui, BridgeShot current,
    [List<BridgeShot> picked = const []]) {
  final all = [current, for (final s in picked) if (s != current) s];
  final clip = current.clip;
  if (clip != null) {
    final clips = [
      for (final s in all)
        if (s.clip case final c?) (layer: s.layer, clip: c),
    ];
    // The clip first, then its layer: a Cut timeline that is open keeps a
    // clip only while its layer is the one selected.
    ui.cutClip.value = (layer: current.layer, clip: clip);
    ui.cutClips.value = clips;
    ui.cutOverlap.value = null;
    final ask = {for (final c in clips) c.clip.toString()};
    ui.cutPickRequest.value = ask;
    // Nobody took it: no Cut timeline is open.
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (identical(ui.cutPickRequest.value, ask)) {
        ui.cutPickRequest.value = null;
      }
    });
  } else {
    ui.cutClip.value = null;
    ui.cutClips.value = const [];
  }
  final layers = <LayerReference>[];
  for (final s in all) {
    if (s.clip == null || s == current) {
      if (!layers.any((l) => l.internallayerId == s.layer.internallayerId)) {
        layers.add(s.layer);
      }
    }
  }
  ui.setSelection(layers);
  final frame = ui.playheadFrame.value;
  if (frame < current.startFrame || frame >= current.endFrame) {
    ui.playheadFrame.value = current.startFrame;
  }
}

/// The shot [delta] along from the current one, or the first and last with
/// [edge]. False when there is nowhere to go.
bool stepShot(LumitUiState ui, int delta, {bool edge = false}) {
  final shots = compShots(ui);
  if (shots.isEmpty) return false;
  final at = currentShotIndex(ui, shots);
  final to = edge
      ? (delta < 0 ? 0 : shots.length - 1)
      : at < 0
          ? 0
          : (at + delta).clamp(0, shots.length - 1);
  if (to == at) return false;
  pickShots(ui, shots[to]);
  return true;
}

/// [shot]'s grade on the effect clipboard. False when it has none.
bool copyShotGrade(LumitUiState ui, GradeShot shot) {
  final ids = shotGradeIds(ui, shot);
  if (ids.isEmpty) return false;
  try {
    ui.copyEffectsToClipboard(shot.layer.copyEffects(effects: ids));
  } catch (_) {
    return false;
  }
  return true;
}

/// The grade of the shot before the current one in time (before [before]
/// when given), or [back] shots before it, in place of the grade on every
/// picked shot or on [onto].
bool applyPreviousGrade(LumitState app, LumitUiState ui,
    {BridgeShot? before, List<GradeShot>? onto, int back = 1}) {
  final shots = compShots(ui);
  final at = before == null
      ? currentShotIndex(ui, shots)
      : shots.indexWhere((s) => shotOf(s).same(shotOf(before)));
  if (at - back < 0) return false;
  final from = shotOf(shots[at - back]);
  final ids = shotGradeIds(ui, from);
  if (ids.isEmpty) {
    // An ungraded shot before: the grade here goes.
    final targets = [
      for (final s in onto ?? gradeTargets(ui))
        (s, shotGradeIds(ui, s).toSet()),
    ];
    asOneUndoStep(app.project, () {
      for (final (s, old) in targets) {
        for (final h in shotHandles(s)) {
          if (old.contains(h.id())) s.layer.removeEffect(effect: h);
        }
      }
    }, name: 'Apply grade');
    _tell(app, ui);
    return true;
  }
  try {
    layGrade(app, ui, from.layer.copyEffects(effects: ids),
        name: 'Apply grade', to: onto);
  } catch (_) {
    return false;
  }
  return true;
}

// --- Keys --------------------------------------------------------------------

/// Run grading [action] from a key. False when it was not one, or could not
/// act, so the key goes on to whatever else wants it.
bool runGradeAction(LumitState app, LumitUiState ui, String action) {
  bool add(GradeNodeKind kind) => addGradeNode(app, ui, kind) != null;
  switch (action) {
    case 'grade.node.serial':
      return add(GradeNodeKind.serial);
    case 'grade.node.parallel':
      return add(GradeNodeKind.parallel);
    case 'grade.node.layer':
      return add(GradeNodeKind.layer);
    case 'grade.node.outside':
      return add(GradeNodeKind.outside);
    case 'grade.matte':
      if (ui.gradeShot.value == null) return false;
      cycleMatteView(ui);
      return true;
    case 'grade.still.grab':
      if (!canGrabStill(ui)) return false;
      grabStill(app, ui);
      return true;
    case 'grade.shot.prev':
      return stepShot(ui, -1);
    case 'grade.shot.next':
      return stepShot(ui, 1);
    case 'grade.shot.first':
      return stepShot(ui, -1, edge: true);
    case 'grade.shot.last':
      return stepShot(ui, 1, edge: true);
    case 'grade.apply.previous':
      return applyPreviousGrade(app, ui);
    case 'grade.apply.second':
      return applyPreviousGrade(app, ui, back: 2);
    case 'grade.lightbox':
      return toggleLightbox(ui);
    case 'grade.compare':
      return toggleGradeCompare(ui);
  }
  if (action.startsWith('grade.memory.')) {
    final slot = int.tryParse(action.substring(action.length - 1));
    if (slot == null) return false;
    return action.startsWith('grade.memory.apply.')
        ? applyMemory(app, ui, slot)
        : storeMemory(app, ui, slot);
  }
  return false;
}
