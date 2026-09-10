// A clip's fades: the curves, and where they run (docs/impl/audio-timeline.md
// §3).
//
// A shape is one curve, written as the gain of a fade *in*: u runs from 0 at
// silence to 1 at full level, and a fade out reads the same curve backwards.
// [clipFadeGain] is the Dart twin of `FadeShape::gain` in
// crates/lumit-core/src/sequence.rs, so the ramp drawn on a lane is the curve
// the mixer plays. Keep the two in step.
//
// The rest is geometry. Where a clip overlaps its neighbour the overlap is the
// crossfade, so the stored seconds are not read there and the pair is drawn
// once; everywhere else a fade is its own seconds, clamped to the clip.
//
// All of it is pure, so it tests without a widget tree and never crosses the
// bridge.

import 'dart:math' as math;

import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'easing_curve.dart';
import 'graph_maths.dart' show CubicSpan, minTangentReach;

/// The gain of a fade **in** at [u]. A fade out is this read backwards,
/// `clipFadeGain(shape, 1 - u)`.
double clipFadeGain(BridgeClipFadeShape shape, double u) {
  final at = u.clamp(0.0, 1.0);
  return switch (shape) {
    BridgeClipFadeShape_Linear() => at,
    BridgeClipFadeShape_Fast() => math.sin(at * math.pi / 2),
    BridgeClipFadeShape_Slow() => 1 - math.cos(at * math.pi / 2),
    BridgeClipFadeShape_Smooth() => at * at * (3 - 2 * at),
    // The exact inverse of Smooth rather than a steeper curve chosen by eye:
    // Smooth is u * u * (3 - 2u), and this is the u it came from.
    BridgeClipFadeShape_Sharp() =>
      0.5 - math.sin(math.asin((1 - 2 * at).clamp(-1.0, 1.0)) / 3),
    BridgeClipFadeShape_Custom(:final x1, :final y1, :final x2, :final y2) =>
      _spanOf(EasingCurve(x1, y1, x2, y2)).valueAt(at),
  };
}

/// A Custom shape as the cubic the graph editor already solves: read at x, not
/// walked in the bezier's own parameter, which is what the engine does too.
CubicSpan _spanOf(EasingCurve curve) {
  final inReach = (1 - curve.x2).clamp(minTangentReach, 1.0);
  return CubicSpan.fromAe(
    0,
    0,
    1,
    1,
    speedOut: curve.y1 / curve.x1,
    inflOut: curve.x1,
    speedIn: (1 - curve.y2) / inReach,
    inflIn: inReach,
  );
}

/// The gain that keeps a pair's power constant: `sqrt(1 - g²)`, which is what
/// *Keep level* puts on the other side of a crossfade.
double powerComplement(double gain) => math.sqrt(math.max(0, 1 - gain * gain));

/// The other side of an overlap, as a gain: the complement of [shape] read
/// backwards, because the two curves run past each other. The partner of a
/// Fast fade is a Fast fade, so an untouched overlap already holds its level.
double keepLevelGain(BridgeClipFadeShape shape, double u) =>
    powerComplement(clipFadeGain(shape, 1 - u));

/// The bezier that draws [gain]: the curve through its height a third and two
/// thirds of the way across.
///
/// With the two handles at x = 1/3 and x = 2/3 the bezier's x is u exactly, so
/// the two remaining numbers fall out of a pair of straight lines. It is a fit,
/// not a conversion - a Custom shape is a cubic and most gains are not - and it
/// is what the editor draws a preset with and what *Keep level* writes for the
/// side it is not dragging.
EasingCurve fitFadeCurve(double Function(double u) gain) {
  final a = gain(1 / 3) - 1 / 27;
  final b = gain(2 / 3) - 8 / 27;
  return EasingCurve(1 / 3, 3 * a - 1.5 * b, 2 / 3, 3 * b - 1.5 * a);
}

/// A shape as the curve the editor draws and drags.
EasingCurve fadeCurveOf(BridgeClipFadeShape shape) => switch (shape) {
      BridgeClipFadeShape_Custom(:final x1, :final y1, :final x2, :final y2) =>
        EasingCurve(x1, y1, x2, y2),
      _ => fitFadeCurve((u) => clipFadeGain(shape, u)),
    };

/// A drawn curve as the shape that is stored.
BridgeClipFadeShape customFadeShape(EasingCurve curve) =>
    BridgeClipFadeShape.custom(
        x1: curve.x1, y1: curve.y1, x2: curve.x2, y2: curve.y2);

/// The five named shapes, in the order every menu shows them. The id is what
/// the name is looked up by, so this file stays pure.
const List<({String id, BridgeClipFadeShape shape})> clipFadeShapes = [
  (id: 'linear', shape: BridgeClipFadeShape.linear()),
  (id: 'fast', shape: BridgeClipFadeShape.fast()),
  (id: 'slow', shape: BridgeClipFadeShape.slow()),
  (id: 'smooth', shape: BridgeClipFadeShape.smooth()),
  (id: 'sharp', shape: BridgeClipFadeShape.sharp()),
];

/// One ramp on a track's lane: the comp frames it runs across, the curve it
/// follows, and which end of its clip it belongs to - [into] rises, the other
/// falls.
typedef ClipFadeRamp = ({
  String clip,
  bool into,
  double from,
  double to,
  BridgeClipFadeShape shape,
});

/// A fade's length while its corner is being dragged, which is what the ramp is
/// drawn from until the drag is let go.
typedef ClipFadeDrag = ({String clip, bool into, double seconds});

/// How long one of a clip's fades is, with a drag in flight applied.
double clipFadeSeconds(BridgeClip clip,
        {required bool into, ClipFadeDrag? live}) =>
    live != null && live.clip == clip.id.toString() && live.into == into
        ? live.seconds
        : (into ? clip.fadeIn : clip.fadeOut).seconds;

/// The clip overlapping [clip] at one of its ends, or null.
///
/// The overlap is the crossfade, so an end inside one takes its length from the
/// overlap rather than from the seconds the clip stores.
BridgeClip? clipFadePartner(List<BridgeClip> clips, BridgeClip clip,
    {required bool into}) {
  final edge = (into ? clip.startFrame : clip.endFrame).toInt();
  for (final other in clips) {
    if (other.id == clip.id) continue;
    if (other.startFrame.toInt() < edge && other.endFrame.toInt() > edge) {
      return other;
    }
  }
  return null;
}

/// Where a fade ends, in comp frames: the clip's own corner when there is no
/// fade, and never past its other end.
double clipFadeEdge(BridgeClip clip,
    {required bool into, required double fps, ClipFadeDrag? live}) {
  final start = clip.startFrame.toInt().toDouble();
  final end = clip.endFrame.toInt().toDouble();
  final frames = clipFadeSeconds(clip, into: into, live: live) * fps;
  return into ? math.min(start + frames, end) : math.max(end - frames, start);
}

/// Which fade the point at [frame] is on, or null for the plain body. An end
/// inside an overlap answers, because the overlap is the crossfade.
bool? clipFadeSideAt(
    List<BridgeClip> clips, BridgeClip clip, double frame, double fps) {
  final over = clipFadePartner(clips, clip, into: true);
  if (over != null && frame <= over.endFrame.toInt()) return true;
  if (over == null && frame <= clipFadeEdge(clip, into: true, fps: fps)) {
    return clip.fadeIn.seconds > 0 ? true : null;
  }
  final under = clipFadePartner(clips, clip, into: false);
  if (under != null && frame >= under.startFrame.toInt()) return false;
  if (under == null && frame >= clipFadeEdge(clip, into: false, fps: fps)) {
    return clip.fadeOut.seconds > 0 ? false : null;
  }
  return null;
}

/// Every ramp a track's lane draws, the overlaps once per pair.
List<ClipFadeRamp> clipFadeRamps(List<BridgeClip> clips, double fps,
    {ClipFadeDrag? live}) {
  final out = <ClipFadeRamp>[];
  for (final clip in clips) {
    final id = clip.id.toString();
    final start = clip.startFrame.toInt().toDouble();
    final end = clip.endFrame.toInt().toDouble();
    final over = clipFadePartner(clips, clip, into: true);
    if (over != null) {
      // The pair, drawn from the incoming clip's start so it is drawn once:
      // the outgoing one falls across the overlap and this one rises across it.
      final to = math.min(over.endFrame.toInt().toDouble(), end);
      out.add((
        clip: over.id.toString(),
        into: false,
        from: start,
        to: to,
        shape: over.fadeOut.shape,
      ));
      out.add((
        clip: id,
        into: true,
        from: start,
        to: to,
        shape: clip.fadeIn.shape,
      ));
    } else {
      final to = clipFadeEdge(clip, into: true, fps: fps, live: live);
      if (to > start) {
        out.add((
          clip: id,
          into: true,
          from: start,
          to: to,
          shape: clip.fadeIn.shape,
        ));
      }
    }
    if (clipFadePartner(clips, clip, into: false) == null) {
      final from = clipFadeEdge(clip, into: false, fps: fps, live: live);
      if (from < end) {
        out.add((
          clip: id,
          into: false,
          from: from,
          to: end,
          shape: clip.fadeOut.shape,
        ));
      }
    }
  }
  return out;
}
