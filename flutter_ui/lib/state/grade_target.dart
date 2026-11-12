// What the Colour workspace is grading: the current shot, the node on it the
// Grade panel edits, and the few choices the grading panels and the Viewer
// share while they work on it.
//
// The shot is the clip picked in the Cut timeline when there is one, else the
// selected layer. Nothing here is written down: it is where the panels are
// looking, not part of the document.

import 'dart:ui' show Offset;

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/grade_shots.dart' show BridgeShot;
import 'package:lumit_flutter/src/rust/api/grade_versions.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:uuid/uuid.dart';

/// The effect a grade node usually is.
const String correctionName = 'correction';

/// The Colour warper, a node with an editor of its own.
const String colourWarperName = 'colour_warper';

/// The effects a grade is made of: everything under Colour that a version
/// keeps. A group's reference is not one.
final Set<String> gradeEffectNames = {
  for (final e in listEffects())
    if (e.category == 'colour' && isGradeEffect(name: e.name)) e.name,
};

/// The grade nodes of a stack, in stack order. Numbered from 1 in this order
/// wherever a node is named by number.
List<BridgeEffectInstanceInfo> gradeNodesOf(
        List<BridgeEffectInstanceInfo> stack) =>
    [
      for (final e in stack)
        if (gradeEffectNames.contains(e.name)) e,
    ];

/// Whether [e] is a shot's reference to a shared node rather than a node of
/// its own: a group grade whose part is Shared.
bool isSharedReference(BridgeEffectInstanceInfo e) =>
    e.name == 'grade_group' &&
    e.values.any((v) =>
        v.id == 'part' &&
        v.value is BridgeEffectValue_Choice &&
        (v.value as BridgeEffectValue_Choice).field0 == 2);

/// The nodes named by number in the node strip and the Grade panel: the
/// grade nodes and the shared nodes, in stack order.
List<BridgeEffectInstanceInfo> gradeNumberedOf(
        List<BridgeEffectInstanceInfo> stack) =>
    [
      for (final e in stack)
        if (gradeEffectNames.contains(e.name) || isSharedReference(e)) e,
    ];

/// One shot: a layer, or with [clip] one clip on a Sequence layer.
@immutable
class GradeShot {
  final LayerReference layer;
  final UuidValue? clip;

  const GradeShot(this.layer, [this.clip]);

  /// The same layer and clip, whatever handle names them.
  bool same(GradeShot? other) =>
      other != null &&
      other.layer.internallayerId == layer.internallayerId &&
      other.clip == clip;

  /// A name for maps that remember something per shot.
  String get key => '${layer.internallayerId}/${clip ?? ''}';
}

/// Which grade the nodes belong to: the shot's own, or a group's, or the
/// comp's.
enum GradeScope { shot, groupPre, groupPost, comp }

/// The Grade panel's pages, in the order its page control shows them.
enum GradePage { wheels, adjust, curves, qualifier, windows, match, look }

/// How the Wheels page shows a wheel's colour: as wheels, bars, the log
/// wheels, or the HDR zones.
enum GradeWheelMode { wheels, bars, log, hdr }

/// What the Viewer's split-screen grid shows: the current shot's versions,
/// the stills picked in the Gallery, or the picked shots.
enum GradeGrid { versions, stills, shots }

/// What the Viewer shows of the current node's key.
enum GradeMatte { off, highlight, key }

/// What Highlight shows where the key is not: grey, black, white, or the
/// picture in black and white.
enum GradeMatteBackdrop { grey, black, white, mono }

/// The qualifier's tools on the picture: take a new range, widen it, or
/// pull it in.
enum GradePick { pick, add, subtract }

/// One stroke a 3D qualifier tool drew on the picture, in comp pixels: what
/// it kept, or with [drop] what it left out. Kept for drawing only; the key
/// is the samples the node holds.
@immutable
class GradeStroke {
  final List<Offset> points;
  final bool drop;
  const GradeStroke(this.points, {this.drop = false});
}

/// How the Viewer lays the reference against the picture: a wipe with an
/// upright divider or one across, the same region side by side, the
/// difference between the two, or one faded over the other.
enum GradeCompare { wipe, wipeAcross, split, difference, mix }

/// A still dragged out of the Gallery: dropped on a shot, it grades it.
@immutable
class GradeStillDrag {
  /// The still's `.lumstill` path.
  final String path;
  const GradeStillDrag(this.path);
}

/// What Match matches the shot to: a still, or another shot at a frame.
@immutable
class GradeReference {
  /// A still's `.lumstill` path and its picture, for a still.
  final String? still;
  final String? stillPicture;

  /// A shot and the comp frame it is read at, for a shot.
  final GradeShot? shot;
  final int frame;

  final String name;

  const GradeReference.still(
      {required String path, required String picture, required this.name})
      : still = path,
        stillPicture = picture,
        shot = null,
        frame = 0;

  const GradeReference.shot(GradeShot this.shot,
      {required this.frame, required this.name})
      : still = null,
        stillPicture = null;
}

/// The grading panels' shared state. Made by `LumitUiState`.
class GradeTarget {
  GradeTarget({
    required ValueListenable<LayerReference?> layer,
    required ValueListenable<({LayerReference layer, UuidValue clip})?> clip,
    required UuidValue? Function() comp,
    required ProjectReference? Function() project,
  })  : _layer = layer,
        _clip = clip,
        _comp = comp,
        _project = project {
    _layer.addListener(_follow);
    _clip.addListener(_follow);
    _follow();
  }

  final ValueListenable<LayerReference?> _layer;
  final ValueListenable<({LayerReference layer, UuidValue clip})?> _clip;

  /// The comp in the Viewer: a clip picked in another one is not the shot.
  final UuidValue? Function() _comp;

  final ProjectReference? Function() _project;

  /// The open project, which keeps the groups and the shared nodes.
  ProjectReference? get project => _project();

  /// The current shot: the picked Cut clip, else the selected layer.
  final ValueNotifier<GradeShot?> shot = ValueNotifier(null);

  /// The effect the Grade panel edits on the current shot. Null leaves the
  /// panel to choose: the last Correction, or a new one on the first touch.
  final ValueNotifier<UuidValue?> node = ValueNotifier(null);

  /// Which grade the nodes are from. Back to the shot's own on a new shot.
  final ValueNotifier<GradeScope> scope = ValueNotifier(GradeScope.shot);

  final ValueNotifier<GradePage> page = ValueNotifier(GradePage.wheels);

  final ValueNotifier<GradeWheelMode> wheelMode =
      ValueNotifier(GradeWheelMode.wheels);

  /// Whether the Shots panel fills the window as a grid of large pictures.
  final ValueNotifier<bool> lightbox = ValueNotifier(false);

  /// What the Viewer's split-screen grid shows, or null while it is off.
  final ValueNotifier<GradeGrid?> grid = ValueNotifier(null);

  /// The stills picked in the Gallery, in its order.
  final ValueNotifier<List<GradeReference>> pickedStills =
      ValueNotifier(const []);

  /// The Viewer's matte view of the current node.
  final ValueNotifier<GradeMatte> matte = ValueNotifier(GradeMatte.off);

  /// Whether the Grade nodes panel shows what each node puts out.
  final ValueNotifier<bool> nodeThumbnails = ValueNotifier(false);

  /// What Highlight shows around the key, until Lumit closes.
  final ValueNotifier<GradeMatteBackdrop> matteBackdrop =
      ValueNotifier(GradeMatteBackdrop.grey);

  /// The qualifier tool armed on the picture, if any.
  final ValueNotifier<GradePick?> pick = ValueNotifier(null);

  /// The 3D qualifier strokes drawn on each node while the project is open,
  /// which the Viewer shows faintly while a tool is armed.
  final ValueNotifier<Map<UuidValue, List<GradeStroke>>> strokes =
      ValueNotifier(const {});

  /// Node [node]'s strokes after a new one: a Pick starts again, Add and
  /// Subtract add to what is there.
  void addStroke(UuidValue node, GradeStroke stroke, {bool fresh = false}) {
    strokes.value = {
      ...strokes.value,
      node: [if (!fresh) ...?strokes.value[node], stroke],
    };
  }

  /// Node [node]'s kept strokes or its dropped ones forgotten.
  void clearStrokes(UuidValue node, {required bool drop}) {
    strokes.value = {
      ...strokes.value,
      node: [
        for (final s in strokes.value[node] ?? const <GradeStroke>[])
          if (s.drop != drop) s
      ],
    };
  }

  /// The window slot whose handles the Viewer shows, 1 to 4, or 0 for none.
  final ValueNotifier<int> window = ValueNotifier(0);

  /// Whether the Viewer shows the colour chart's grid.
  final ValueNotifier<bool> chart = ValueNotifier(false);

  /// The chart's four corners in picture pixels, from the top left round
  /// clockwise, while the Viewer shows it. Null until it is placed.
  final ValueNotifier<List<Offset>?> chartCorners = ValueNotifier(null);

  /// What the Viewer compares against, which Match can match to.
  final ValueNotifier<GradeReference?> reference = ValueNotifier(null);

  /// Whether the Viewer shows the reference now. A reference stays set while
  /// this is off, so it can be put back.
  final ValueNotifier<bool> comparing = ValueNotifier(false);

  final ValueNotifier<GradeCompare> compareStyle =
      ValueNotifier(GradeCompare.wipe);

  /// The reference on the left (or on top) and the picture after it.
  final ValueNotifier<bool> compareSwapped = ValueNotifier(false);

  /// Where the divider sits, as a fraction across (down for a wipe across).
  /// In a mix it is how much of the reference shows.
  final ValueNotifier<double> compareAt = ValueNotifier(0.5);

  /// Compare against [to] in the Viewer, or stop with null.
  void compareWith(GradeReference? to) {
    reference.value = to;
    comparing.value = to != null;
  }

  /// The chart's corners on each shot it was placed on, while the project is open.
  final Map<String, List<Offset>?> _charts = {};

  /// The Gallery's open album, where Grab still puts a still. Null puts it
  /// in the first album there is.
  final ValueNotifier<String?> album = ValueNotifier(null);

  /// The node picked on each shot while the project is open, so going back to a shot
  /// finds the node that was left.
  final Map<String, UuidValue?> _nodes = {};

  void _follow() {
    final clip = _clip.value;
    final layer = _layer.value;
    // A clip left picked after another layer was chosen is not the shot.
    final picked = clip != null &&
            clip.layer.internalcompId == _comp() &&
            clip.layer.internallayerId == layer?.internallayerId
        ? clip
        : null;
    final next = picked != null
        ? GradeShot(picked.layer, picked.clip)
        : layer == null
            ? null
            : GradeShot(layer);
    final was = shot.value;
    if (next == null ? was == null : next.same(was)) return;
    if (was != null) _charts[was.key] = chartCorners.value;
    chartCorners.value = next == null ? null : _charts[next.key];
    shot.value = next;
    node.value = next == null ? null : _nodes[next.key];
    scope.value = GradeScope.shot;
    window.value = 0;
    pick.value = null;
  }

  // --- The shot and the playhead ---------------------------------------------

  ValueNotifier<int>? _playhead;
  late List<BridgeShot> Function() _shots;
  late bool Function() _grading;
  late void Function(BridgeShot shot) _pick;

  /// Set while one side moves the other, so neither answers back.
  bool _linking = false;

  /// Keep the current shot and the playhead together while [grading] says a
  /// Colour panel is in the workspace, as a colourist's current clip is the
  /// one under the playhead. A new shot the playhead is outside brings the
  /// playhead to its first frame; the playhead moved out of the shot makes
  /// the shot under it current, through [pick].
  void linkPlayhead({
    required ValueNotifier<int> playhead,
    required List<BridgeShot> Function() shots,
    required bool Function() grading,
    required void Function(BridgeShot shot) pick,
  }) {
    _playhead = playhead;
    _shots = shots;
    _grading = grading;
    _pick = pick;
    shot.addListener(_shotMoved);
    playhead.addListener(_playheadMoved);
  }

  static bool _holds(BridgeShot s, int frame) =>
      frame >= s.startFrame && frame < s.endFrame;

  BridgeShot? _listed(List<BridgeShot> shots, GradeShot current) => shots
      .where((s) => GradeShot(s.layer, s.clip).same(current))
      .firstOrNull;

  void _shotMoved() {
    final playhead = _playhead;
    final current = shot.value;
    if (_linking || playhead == null || current == null || !_grading()) {
      return;
    }
    final shots = _shots();
    final s = _listed(shots, current);
    if (s == null || _holds(s, playhead.value)) return;
    // A clip that starts under the end of the one before it, in a
    // transition, starts where it is seen on its own.
    var frame = s.startFrame;
    for (final o in shots) {
      if (o.clip != null &&
          o.clip != s.clip &&
          o.layer.internallayerId == s.layer.internallayerId &&
          _holds(o, frame)) {
        frame = o.endFrame;
      }
    }
    _linking = true;
    try {
      playhead.value = frame < s.endFrame ? frame : s.startFrame;
    } finally {
      _linking = false;
    }
  }

  void _playheadMoved() {
    final playhead = _playhead;
    final current = shot.value;
    if (_linking || playhead == null || current == null || !_grading()) {
      return;
    }
    final frame = playhead.value;
    final shots = _shots();
    final s = _listed(shots, current);
    if (s == null || _holds(s, frame)) return;
    // A clip gives way to the clip under the playhead on its own layer; a
    // layer to the topmost layer there.
    final under = [
      for (final o in shots)
        if (_holds(o, frame) &&
            (current.clip == null
                ? o.clip == null
                : o.clip != null &&
                    o.layer.internallayerId == current.layer.internallayerId))
          o,
    ]..sort((a, b) => current.clip == null
        ? a.row.compareTo(b.row)
        : b.startFrame.compareTo(a.startFrame));
    final next = under.firstOrNull;
    if (next == null) return;
    _linking = true;
    try {
      _pick(next);
    } finally {
      _linking = false;
    }
  }

  /// Make [id] the current node on the current shot, and remember it there.
  void setNode(UuidValue? id) {
    final current = shot.value;
    if (current != null) _nodes[current.key] = id;
    node.value = id;
  }

  void dispose() {
    _layer.removeListener(_follow);
    _clip.removeListener(_follow);
    shot.removeListener(_shotMoved);
    _playhead?.removeListener(_playheadMoved);
    for (final n in <ChangeNotifier>[
      shot,
      node,
      scope,
      page,
      wheelMode,
      lightbox,
      grid,
      pickedStills,
      matte,
      matteBackdrop,
      nodeThumbnails,
      pick,
      strokes,
      window,
      chart,
      chartCorners,
      reference,
      comparing,
      compareStyle,
      compareSwapped,
      compareAt,
      album,
    ]) {
      n.dispose();
    }
  }
}
