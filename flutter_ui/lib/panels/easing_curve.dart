// The easing curve: one normalised bezier shape, reusable across keyframes
// (K-348). Pure, so it unit-tests without a widget tree and never crosses the
// bridge.
//
// In plain terms: the graph editor shapes one span at a time, by dragging the
// tangent handles of two particular keyframes. An *easing curve* is the same
// shape held apart from any one span — drawn once in a unit box, then stamped
// onto every span the selection covers. It is the shape editors mean when they
// say "put my ease on these keyframes".
//
// **Why the stored numbers differ per span.** A keyframe stores its side as
// AE-style *speed* (value-units per second) and *influence* (how far the handle
// reaches, as a fraction of the gap) — see `crates/lumit-core/src/anim.rs`.
// Speed is an absolute rate, so the same drawn shape becomes a different speed
// on a span that covers 400 pixels than on one that covers 40. Influence is
// already a fraction, so it carries across untouched. [EasingCurve.sidesFor]
// is that conversion, and it is the whole reason this file exists.

import 'package:lumit_flutter/src/rust/api/effect.dart';

import 'graph_maths.dart' show minTangentReach;

/// How far past the unit box a control point may reach vertically, in box
/// heights — so y lives in −0.5 … 1.5.
///
/// Unlike the horizontal bound this one is not a correctness rule; overshoot is
/// perfectly legal to store, and the engine would evaluate any speed given to
/// it. It is a *reachability* rule: the editor draws a fixed window, and a
/// handle dragged past the edge of it becomes a handle that cannot be seen, and
/// so cannot be dragged back. The editor sizes its drawing area from this
/// constant, which is why the two agree by construction.
const double easingHandleReach = 0.5;

/// A normalised easing shape: the cubic bezier from (0, 0) to (1, 1) whose two
/// control points are [x1], [y1] and [x2], [y2] — the same four numbers CSS
/// writes as `cubic-bezier(x1, y1, x2, y2)`.
///
/// **The two axes are bounded for different reasons.** x is time, and
/// docs/impl/keyframe-eval.md §1 keeps the curve x-monotone by holding both
/// control points inside the span — that bound is correctness. y is value, and
/// overshooting it is the point of a bouncy ease, so y is free to leave the
/// box; it stops only at [easingHandleReach], where the editor's own view ends.
class EasingCurve {
  /// The out-side control point of the span's first key.
  final double x1;
  final double y1;

  /// The in-side control point of the span's second key.
  final double x2;
  final double y2;

  /// Both control points clamped to a legal, reachable shape: x inside the span
  /// and never quite touching its own end, y inside the editor's view.
  EasingCurve(double x1, double y1, double x2, double y2)
      : x1 = x1.clamp(minTangentReach, 1.0),
        y1 = y1.clamp(-easingHandleReach, 1 + easingHandleReach),
        x2 = x2.clamp(0.0, 1 - minTangentReach),
        y2 = y2.clamp(-easingHandleReach, 1 + easingHandleReach);

  /// The two keyframe sides this shape becomes on a span whose chord slope is
  /// [chordSlope] (Δvalue ÷ Δtime, in value-units per second).
  ///
  /// The mapping falls straight out of the control points in
  /// docs/impl/keyframe-eval.md §1. Writing the span's out-side influence as
  /// `b1` and speed as `s1`, that note places the first control point at
  /// `(t1 + b1·Δt, v1 + s1·b1·Δt)`, while this curve places it at
  /// `(t1 + x1·Δt, v1 + y1·Δv)`. Equating the two coordinates gives
  /// `b1 = x1` and `s1 = (y1 / x1)·(Δv / Δt)`, and the in-side follows the same
  /// way from the far end, where the reach is measured back from `t2`.
  ///
  /// A flat span has `chordSlope` 0, and every speed with it: a shape stamped
  /// on a span that does not move still does not move.
  ({BridgeSideInterp out, BridgeSideInterp inTo}) sidesFor(double chordSlope) {
    // Reach measured back from the far end, so the in-side's fraction is what
    // is left of the span rather than x2 itself.
    //
    // The clamp restates the constructor's guarantee at the point that depends
    // on it: this is the divisor two lines down, and influence is the number
    // actually stored, so both want the bound held here rather than inferred
    // from a bound on x2 several lines up. It does not currently bite — the
    // narrowest legal reach is 1 − 0.999, which is 0.001000000000000001 and so
    // already clear of the limit.
    final inReach = (1 - x2).clamp(minTangentReach, 1.0);
    return (
      out: BridgeSideInterp.bezier(BridgeBezierSide(
        speed: y1 / x1 * chordSlope,
        influence: x1,
      )),
      inTo: BridgeSideInterp.bezier(BridgeBezierSide(
        speed: (1 - y2) / inReach * chordSlope,
        influence: inReach,
      )),
    );
  }

  /// This curve with one control point moved, for the editor's drag.
  EasingCurve withHandle({
    required bool first,
    required double x,
    required double y,
  }) =>
      first ? EasingCurve(x, y, x2, y2) : EasingCurve(x1, y1, x, y);

  /// The curve's y at parameter [u] — the shape as drawn, for the preview
  /// stroke. Parametric in u rather than solved for x: the editor draws the
  /// curve, it does not sample it at particular times, so the root-solve the
  /// engine needs (docs/impl/keyframe-eval.md §2) would buy nothing here.
  double yAt(double u) {
    final v = 1 - u;
    return 3 * v * v * u * y1 + 3 * v * u * u * y2 + u * u * u;
  }

  /// The curve's x at parameter [u]; paired with [yAt] to walk the stroke.
  double xAt(double u) {
    final v = 1 - u;
    return 3 * v * v * u * x1 + 3 * v * u * u * x2 + u * u * u;
  }

  @override
  bool operator ==(Object other) =>
      other is EasingCurve &&
      other.x1 == x1 &&
      other.y1 == y1 &&
      other.x2 == x2 &&
      other.y2 == y2;

  @override
  int get hashCode => Object.hash(x1, y1, x2, y2);

  @override
  String toString() => 'EasingCurve($x1, $y1, $x2, $y2)';
}

/// A preset shape and the id its name is looked up by. The name itself lives in
/// the l10n table, not here: this file stays pure so the maths unit-tests
/// without a locale (the same reason `graph_maths.dart` holds no strings).
class EasingPreset {
  final String id;
  final EasingCurve curve;
  const EasingPreset(this.id, this.curve);
}

/// The shipped shapes, gentlest first, ending with the two that leave the box.
///
/// The first is the F9 easy ease exactly: speed 0 at both ends, influence one
/// third — the same constant as [easyEase], drawn rather than stated.
///
/// **Named by what they do, not by "in" and "out".** Those two words already
/// mean a *side* here — `ease in` touches the in side, the way F9 does — while
/// the web's `ease-in` means a slow start, which is the other side entirely. A
/// preset row using either sense would be read in the other. So the names say
/// which end of the travel is slow, and the ambiguity never arises.
///
/// A side drawn *on the chord* (handle at 1/3 along the diagonal) is what
/// `anim.rs` calls a linear side, so "slow start" really is flat out of the
/// first key and straight into the second.
final List<EasingPreset> easingPresets = [
  EasingPreset('easy', EasingCurve(1 / 3, 0, 2 / 3, 1)),
  EasingPreset('slowStart', EasingCurve(1 / 3, 0, 2 / 3, 2 / 3)),
  EasingPreset('slowFinish', EasingCurve(1 / 3, 1 / 3, 2 / 3, 1)),
  EasingPreset('heavy', EasingCurve(0.77, 0, 0.175, 1)),
  EasingPreset('snap', EasingCurve(0.16, 1, 0.3, 1)),
  EasingPreset('overshoot', EasingCurve(0.34, 1.5, 0.64, 1)),
  EasingPreset('anticipate', EasingCurve(0.36, -0.5, 0.66, 1)),
];
