// The grading calls the engine answers: nodes that need the stack to know
// about them, the Viewer's matte view, the solvers and window tracking.
//
// The Grade panel and the panels beside it ask through this one class, so
// each call has one place where it meets the bridge. A call whose engine side
// isn't in this build says so through its `ready` flag, and the control that
// needs it stands down rather than doing nothing when pressed.

import 'dart:ui' show Offset;

import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/grade_matte.dart';
import 'package:lumit_flutter/src/rust/api/grade_solve.dart';
import 'package:lumit_flutter/src/rust/api/grade_track.dart';
import 'package:uuid/uuid.dart';

import 'grade_target.dart';

/// How a new node meets the one it is added after.
enum GradeNodeKind { serial, parallel, layer, outside }

/// Which way a window is tracked from the playhead.
enum GradeTrackDirection { backward, forward }

/// A circle on the picture, in comp pixels: the samples a pick reads.
class GradeCircle {
  final Offset centre;
  final double radius;
  const GradeCircle(this.centre, this.radius);
}

/// Where a window track has got to.
class GradeTrackProgress {
  /// Frames tracked so far, of [total].
  final int done, total;

  /// The comp frame tracking was lost at, when it was.
  final int? lostAt;
  const GradeTrackProgress(
      {required this.done, required this.total, this.lostAt});
}

/// What Match chart found: the rows to write, and the average colour error
/// over the patches before and after, in delta E 2000.
class GradeChartResult {
  final Map<String, BridgeEffectValue> rows;
  final double errorBefore, errorAfter;
  const GradeChartResult(
      {required this.rows,
      required this.errorBefore,
      required this.errorAfter});
}

/// The rows a solver hands back, by Correction row id, for the panel to
/// write into the node as one step.
typedef GradeRows = Map<String, BridgeEffectValue>;

class GradeEngine {
  const GradeEngine();

  // --- Nodes ---------------------------------------------------------------

  /// Whether [addNode] can make [kind] on [shot]. An outside node needs a
  /// key wire, which only a layer's stack carries, not a clip's.
  bool canAdd(GradeNodeKind kind, [GradeShot? shot]) =>
      kind != GradeNodeKind.outside || shot?.clip == null;

  /// Add a Correction to [shot] right after [after] (first in the stack when
  /// null), meeting it as [kind] says. Each is one undo step. An outside
  /// node's Matte takes [after]'s key, inverted. Answers the new node's id, or
  /// null when nothing was added.
  UuidValue? addNode(GradeShot shot, UuidValue? after, GradeNodeKind kind) {
    if (!canAdd(kind, shot)) return null;
    final layer = shot.layer;
    final clip = shot.clip;
    try {
      if (after != null) {
        return switch (kind) {
          GradeNodeKind.serial =>
            layer.addSerialGradeNode(clip: clip, after: after),
          GradeNodeKind.parallel =>
            layer.addParallelGradeNode(clip: clip, after: after),
          GradeNodeKind.layer =>
            layer.addLayerGradeNode(clip: clip, after: after),
          GradeNodeKind.outside => layer.addOutsideGradeNode(after: after),
        };
      }
      // Nothing to come after: a plain Correction at the top of the stack.
      if (kind == GradeNodeKind.outside) return null;
      final before = {for (final e in _stack(shot)) e.id()};
      clip == null
          ? layer.addEffect(name: correctionName)
          : layer.addClipEffect(clip: clip, name: correctionName);
      final made =
          _stack(shot).where((h) => !before.contains(h.id())).firstOrNull;
      if (made == null) return null;
      layer.reorderEffect(effect: made, newIndex: 0);
      return made.id();
    } catch (_) {
      // A locked layer, or a node that has gone.
      return null;
    }
  }

  List<BridgeEffectInstance> _stack(GradeShot shot) => shot.clip == null
      ? shot.layer.getEffects()
      : shot.layer.getClipEffects(clip: shot.clip!, onCompClock: true);

  // --- Matte view ----------------------------------------------------------

  bool get matteReady => true;

  /// Show [node]'s key on [shot] in Viewer view [view] as [mode] says,
  /// Highlight over [backdrop], or the picture again with [GradeMatte.off].
  /// Preview only: nothing is written. The caller asks for the frame again
  /// afterwards.
  void setMatteView(CompositionReference comp, GradeShot shot, UuidValue? node,
      GradeMatte mode,
      {required int view,
      GradeMatteBackdrop backdrop = GradeMatteBackdrop.grey}) {
    try {
      comp.setMatteView(
          view: view,
          mode: node == null
              ? BridgeMatteView.off
              : switch (mode) {
                  GradeMatte.off => BridgeMatteView.off,
                  GradeMatte.highlight => BridgeMatteView.highlight,
                  GradeMatte.key => BridgeMatteView.key,
                },
          layer: shot.layer.internallayerId,
          clip: shot.clip,
          effect: node ?? shot.layer.internallayerId,
          backdrop: switch (backdrop) {
            GradeMatteBackdrop.grey => BridgeMatteBackdrop.grey,
            GradeMatteBackdrop.black => BridgeMatteBackdrop.black,
            GradeMatteBackdrop.white => BridgeMatteBackdrop.white,
            GradeMatteBackdrop.mono => BridgeMatteBackdrop.mono,
          });
    } catch (_) {
      // No worker yet. The next change asks again.
    }
  }

  // --- Solvers -------------------------------------------------------------

  bool get solversReady => true;

  /// Neutralise the cast at comp [frame] and set the black and white points:
  /// temperature, tint, lift and gain.
  Future<GradeRows?> autoBalance(CompositionReference comp, GradeShot shot,
          int frame, UuidValue node) async =>
      _rows(await comp.autoBalance(
          layer: shot.layer.internallayerId,
          clip: shot.clip,
          frame: frame,
          node: node));

  /// Lift, gamma, gain and saturation that bring [shot] at [frame] to [to].
  Future<GradeRows?> matchShot(CompositionReference comp, GradeShot shot,
      int frame, UuidValue node, GradeReference to) async {
    final still = to.still;
    final other = to.shot;
    if (still != null) {
      return _rows(await comp.matchStill(
          layer: shot.layer.internallayerId,
          clip: shot.clip,
          frame: frame,
          node: node,
          still: still));
    }
    if (other == null) return null;
    return _rows(await comp.matchShot(
        layer: shot.layer.internallayerId,
        clip: shot.clip,
        frame: frame,
        node: node,
        refLayer: other.layer.internallayerId,
        refClip: other.clip,
        refFrame: to.frame));
  }

  /// A colour chart read inside [corners] (comp pixels, top left round
  /// clockwise): space, the mixer rows and exposure, written into [node].
  Future<GradeChartResult?> matchChart(CompositionReference comp,
      GradeShot shot, int frame, UuidValue node, List<Offset> corners) async {
    if (corners.length != 4) return null;
    final got = await comp.matchChart(
        layer: shot.layer.internallayerId,
        clip: shot.clip,
        frame: frame,
        node: node,
        corners: [for (final c in corners) ...[c.dx, c.dy]]);
    double mean(List<double> v) =>
        v.isEmpty ? 0 : v.reduce((a, b) => a + b) / v.length;
    return GradeChartResult(
        rows: _rows(got.rows),
        errorBefore: mean(got.before),
        errorAfter: mean(got.after));
  }

  /// The qualifier rows that take in, widen to or pull in from the samples
  /// inside [circle], by [mode], from the node's input picture.
  Future<GradeRows?> qualifierFromPick(CompositionReference comp,
          GradeShot shot, int frame, UuidValue node, GradeCircle circle,
          GradePick mode) async =>
      _rows(await comp.pickQualifier(
          layer: shot.layer.internallayerId,
          clip: shot.clip,
          frame: frame,
          node: node,
          x: circle.centre.dx,
          y: circle.centre.dy,
          radius: circle.radius,
          mode: switch (mode) {
            GradePick.pick => BridgePickMode.set_,
            GradePick.add => BridgePickMode.add,
            GradePick.subtract => BridgePickMode.subtract,
          }));

  /// The 3D qualifier rows that keep, add or drop the colours along a stroke
  /// through [points] (comp pixels), [radius] wide, by [mode]. The node is
  /// switched to 3D.
  Future<GradeRows?> qualifier3dFromStroke(
          CompositionReference comp,
          GradeShot shot,
          int frame,
          UuidValue node,
          List<Offset> points,
          double radius,
          GradePick mode) async =>
      _rows(await comp.pickQualifier3D(
          layer: shot.layer.internallayerId,
          clip: shot.clip,
          frame: frame,
          node: node,
          points: [for (final p in points) ...[p.dx, p.dy]],
          radius: radius,
          mode: _pickMode(mode)));

  static BridgePickMode _pickMode(GradePick mode) => switch (mode) {
        GradePick.pick => BridgePickMode.set_,
        GradePick.add => BridgePickMode.add,
        GradePick.subtract => BridgePickMode.subtract,
      };

  /// The hue (or saturation, or luma, by [curve]'s input) at the middle of
  /// [circle] and how far it spreads, both 0 to 1, for a hue curve's three
  /// points.
  Future<(double, double)?> hueCurvePick(CompositionReference comp,
      GradeShot shot, int frame, UuidValue node, GradeCircle circle,
      String curve) async {
    final got = await comp.pickHueCurve(
        layer: shot.layer.internallayerId,
        clip: shot.clip,
        frame: frame,
        node: node,
        x: circle.centre.dx,
        y: circle.centre.dy,
        radius: circle.radius,
        curve: curve);
    return (got.centre, got.spread);
  }

  static GradeRows _rows(List<BridgeSolvedRow> rows) =>
      {for (final r in rows) r.id: r.value};

  // --- Window tracking -----------------------------------------------------

  bool get trackingReady => true;

  /// The node being tracked, so [stopTracking] knows what to stop.
  static UuidValue? _tracking;

  /// Track window [slot] (1 to 4) of [node] from comp [frame] to the end of
  /// the shot, or with [oneFrame] a frame at a time, keying its centre, size
  /// and angle as one undo step. The stream ends when the track does.
  Stream<GradeTrackProgress> trackWindow(
      CompositionReference comp,
      GradeShot shot,
      UuidValue node,
      int slot,
      int frame,
      GradeTrackDirection direction,
      {bool oneFrame = false}) {
    _tracking = node;
    final run = switch ((direction, oneFrame)) {
      (GradeTrackDirection.forward, false) => BridgeTrackRun.forwards,
      (GradeTrackDirection.backward, false) => BridgeTrackRun.backwards,
      (GradeTrackDirection.forward, true) => BridgeTrackRun.nextFrame,
      (GradeTrackDirection.backward, true) => BridgeTrackRun.previousFrame,
    };
    return shot.layer
        .trackGradeWindow(
            clip: shot.clip, effect: node, slot: slot, frame: frame, run: run)
        .map((s) => GradeTrackProgress(
            done: s.done, total: s.total, lostAt: s.lost));
  }

  /// Stop the track in hand. The frames done so far are kept.
  void stopTracking() {
    final node = _tracking;
    if (node != null) cancelWindowTrack(effect: node);
    _tracking = null;
  }
}
