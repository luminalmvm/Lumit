// Snapping in the Timeline (docs/07-UI-SPEC.md §4.5).
//
// **In plain terms.** While you drag something along the Timeline it should
// want to land on the things already there — the start of a layer, a keyframe,
// a marker, the playhead — rather than a hair away from them. That wanting is
// snapping, and this file is the arithmetic of it: given where the pointer says
// the thing is, and everything it could land on, where does it actually go?
//
// **Two rules the spec fixes, and both matter.**
//
// *Distance is measured in screen pixels, never in time.* Zoomed out, a
// hundred frames may be ten pixels apart and snapping should be eager; zoomed
// in, one frame may be fifty pixels and it should not reach across three of
// them. Measuring in pixels makes the magnification the precision control,
// which is what makes the same slop feel right at every zoom.
//
// *What caught the drag is reported, not just where it landed.* The spec
// requires the target to be indicated at the moment of capture — otherwise a
// drag that jumps looks like a bug rather than a service. So the result carries
// the target, and the caller draws it.
//
// All of it is pure, so all of it is tested against hand-computed cases rather
// than by dragging in a widget tree.

import 'package:flutter/foundation.dart';

import '../src/rust/api/composition.dart';
import 'graph_maths.dart' show rationalSeconds;

/// What a snap landed on. The kind is for the *indicator* — a snap to a marker
/// and a snap to the playhead are the same arithmetic and different news.
enum SnapKind {
  /// A layer's first or last frame.
  layerIn,
  layerOut,

  /// A cut inside a Sequence layer: where one clip ends and the next begins.
  editPoint,

  /// Another keyframe, on any row.
  keyframe,

  /// A composition or layer marker. **Beat markers are markers** — beat
  /// detection writes ordinary markers, so they are snap targets by being
  /// markers rather than by being a separate kind (docs/09-AUDIO.md).
  marker,
  playhead,
  workAreaStart,
  workAreaEnd,
}

/// One thing a drag can land on, at [frame] in comp frames.
@immutable
class SnapTarget {
  final double frame;
  final SnapKind kind;

  const SnapTarget(this.frame, this.kind);

  @override
  bool operator ==(Object other) =>
      other is SnapTarget && other.frame == frame && other.kind == kind;

  @override
  int get hashCode => Object.hash(frame, kind);

  @override
  String toString() => 'SnapTarget($frame, ${kind.name})';
}

/// Where a drag landed, and what caught it — null when nothing did, which is
/// what tells the caller to draw no indicator.
typedef SnapResult = ({double frame, SnapTarget? caught});

/// How near, in screen pixels, a drag has to come before a target takes it.
///
/// Eight is a little under half a row's height: close enough that landing on a
/// marker takes no aim, far enough that a frame either side of it is still
/// reachable at any useful zoom.
const double snapSlopPixels = 8;

/// Where `frame` should land.
///
/// With [magnet] off nothing moves: the frame is returned as the pointer put
/// it, which is what lets a key sit *between* frames when that is wanted
/// (docs/07 §4.5 — the time stays an exact rational either way, it is simply
/// not a whole number of frames).
///
/// With it on, the nearest target within [snapSlopPixels] of the pointer wins
/// and the frame lands exactly on it. Failing that the drag falls back to the
/// whole frame, which is the magnet's original and much narrower behaviour
/// (K-190) and stays the answer when there is nothing else nearby.
///
/// [perFrame] is the axis's pixels-per-frame, and is what turns the pixel slop
/// into a frame distance. A zero or negative one (a collapsed axis) snaps to
/// whole frames and reaches for nothing, rather than dividing by it.
SnapResult snapFrame({
  required double frame,
  required Iterable<SnapTarget> targets,
  required double perFrame,
  required bool magnet,
  double slopPx = snapSlopPixels,
}) {
  if (!magnet) return (frame: frame, caught: null);
  if (perFrame <= 0) return (frame: frame.roundToDouble(), caught: null);

  SnapTarget? best;
  var bestPx = slopPx;
  for (final target in targets) {
    final px = ((target.frame - frame) * perFrame).abs();
    // Strictly nearer, so the first of two equally close targets keeps it and
    // the answer does not depend on the order a caller happened to gather them.
    if (px < bestPx) {
      bestPx = px;
      best = target;
    }
  }
  if (best != null) return (frame: best.frame, caught: best);
  return (frame: frame.roundToDouble(), caught: null);
}

/// Whether a drag in flight should ignore snapping this instant.
///
/// `Ctrl` held suspends it (docs/07 §4.5) — the escape hatch for the one time
/// in ten that the thing you want is exactly where a snap will not let you put
/// it. Held rather than toggled, because it is wanted for a moment inside a
/// gesture rather than for a session.
bool snapSuspended({required bool controlPressed}) => controlPressed;

/// One lane's keys, as the gatherer wants them: the row's id (so the lane
/// being dragged can leave its own keys out) and the frames its diamonds sit
/// on.
typedef SnapKeyRow = ({String rowId, List<double> frames});

/// Everything on the Timeline a drag could land on, gathered from the read
/// model (K-184) — so building the list costs no bridge calls.
///
/// The spec's list (docs/07 §4.5): **edit points** (the cuts inside a Sequence
/// layer), **layer in/out points**, **keyframes**, **markers** (composition and
/// layer, beat markers among them), **the playhead**, and the **work area
/// edges**.
///
/// [exceptRow] drops one lane's keys, so a key being dragged does not snap to
/// itself — which would pin it where it started and look like a broken drag.
///
/// Everything is in comp frames, which is what the lanes are drawn in.
List<SnapTarget> snapTargetsOf({
  required List<BridgeLayerEntry> layers,
  required List<BridgeMarker> compMarkers,
  required List<SnapKeyRow> keyRows,
  required int playheadFrame,
  required ({int start, int end, bool whole}) work,
  required double fps,
  String? exceptRow,
}) {
  final out = <SnapTarget>[
    SnapTarget(playheadFrame.toDouble(), SnapKind.playhead),
  ];
  // The work area's edges, unless it is the whole comp — in which case they are
  // the comp's own ends and snapping to them says nothing.
  if (!work.whole) {
    out
      ..add(SnapTarget(work.start.toDouble(), SnapKind.workAreaStart))
      ..add(SnapTarget(work.end.toDouble(), SnapKind.workAreaEnd));
  }
  for (final marker in compMarkers) {
    out.add(SnapTarget(rationalSeconds(marker.time) * fps, SnapKind.marker));
  }
  for (final entry in layers) {
    final info = entry.info;
    out
      ..add(SnapTarget(info.inFrame.toDouble(), SnapKind.layerIn))
      ..add(SnapTarget(info.outFrame.toDouble(), SnapKind.layerOut));
    for (final m in info.markers) {
      out.add(SnapTarget(m.frame.toDouble(), SnapKind.marker));
    }
    // A Sequence layer's cuts. A clip's placement is measured from the layer's
    // own start, so it is offset by the in point to reach comp frames; the last
    // clip's end is the layer's out point, already in the list.
    for (final clip in info.clips) {
      final start = rationalSeconds(clip.placeStart) * fps;
      out.add(SnapTarget(info.inFrame + start, SnapKind.editPoint));
    }
  }
  for (final row in keyRows) {
    if (row.rowId == exceptRow) continue;
    for (final frame in row.frames) {
      out.add(SnapTarget(frame, SnapKind.keyframe));
    }
  }
  return out;
}
