// The graph editor's maths, all pure so it unit-tests without a widget tree
// and never crosses the bridge from a build or a paint.
//
// In plain terms: this file knows how to read a keyframe curve the exact way
// the engine does (`crates/lumit-core/src/anim.rs`, ported line for line from
// the binding algorithm in docs/impl/keyframe-eval.md), how a tangent handle's
// position on screen maps to the speed/influence pair a keyframe stores, how
// to frame a set of curves vertically so everything is in view, and how to
// write and read the After Effects keyframe clipboard text.
//
// **Why a Dart copy of the engine's evaluator exists at all.** The painter
// samples the curve at every few pixels; asking the engine per sample would be
// hundreds of bridge calls per paint (forbidden — see
// bridge_call_budget_test.dart). The maths is ~50 flops, the algorithm is
// pinned by the impl note on both sides, and the golden tests hold the two
// implementations together.

import 'dart:math' as math;

import 'package:lumit_flutter/src/rust/api/effect.dart';

/// A rational time as plain seconds — the evaluation domain (docs/14 §2:
/// authoritative times stay rational; evaluation converts once).
double rationalSeconds(BridgeRational r) => r.num / r.den.toDouble();

/// The AE easy-ease constant (F9): speed 0, influence one third.
const BridgeSideInterp easyEase = BridgeSideInterp.bezier(
  BridgeBezierSide(speed: 0, influence: 1 / 3),
);

/// `keys` in time order — what [evaluateKeys] and [evaluateKeysSpeed] both
/// require, and what a drag in flight can break.
///
/// **Why a list of keys is ever out of order.** A key drag holds its index for
/// the length of the gesture: the selection, the glyph loop and the tangent
/// handles are all indexed into the shown list, and re-sorting under them
/// mid-drag would re-aim them at each other's keys. So a marquee of selected
/// keys dragged past a key it is *not* moving leaves the list in the moved
/// order, not in time order — and the evaluator, which walks spans assuming
/// time order, then drew nothing across the crossed key while the drag ran.
/// The *drawing* wants time order and has no indices to protect, so it sorts
/// here and the gesture's own bookkeeping is left alone.
///
/// The list is returned untouched when it is already in order, which is every
/// paint but the ones during such a crossing.
List<BridgeKeyframe> keysInTimeOrder(List<BridgeKeyframe> keys) {
  for (var i = 1; i < keys.length; i++) {
    if (rationalSeconds(keys[i - 1].time) <= rationalSeconds(keys[i].time)) {
      continue;
    }
    return [...keys]..sort(
        (a, b) => rationalSeconds(a.time).compareTo(rationalSeconds(b.time)));
  }
  return keys;
}

/// The keys of a scalar, or empty for a static one.
List<BridgeKeyframe> keysOf(BridgeScalar scalar) => switch (scalar) {
      BridgeScalar_Keyframed(:final field0) => field0,
      BridgeScalar_Static() => const [],
      BridgeScalar_Expression() => const [],
    };

// ---------------------------------------------------------------------------
// The engine's evaluator, ported (anim.rs — keep the two in step).
// ---------------------------------------------------------------------------

/// One span's cubic, built from AE parameters (docs/impl/keyframe-eval.md §1):
/// P0=(t1,v1)  P1=(t1+b1·Δt, v1+s1·b1·Δt)  P2=(t2−b2·Δt, v2−s2·b2·Δt)  P3=(t2,v2)
class CubicSpan {
  final List<double> x;
  final List<double> y;

  CubicSpan.fromAe(
    double t1,
    double v1,
    double t2,
    double v2, {
    required double speedOut,
    required double inflOut,
    required double speedIn,
    required double inflIn,
  })  : x = [t1, t1 + inflOut * (t2 - t1), t2 - inflIn * (t2 - t1), t2],
        y = [
          v1,
          v1 + speedOut * inflOut * (t2 - t1),
          v2 - speedIn * inflIn * (t2 - t1),
          v2,
        ];

  static double _bezier(List<double> p, double u) {
    final w = 1.0 - u;
    return w * w * w * p[0] +
        3.0 * w * w * u * p[1] +
        3.0 * w * u * u * p[2] +
        u * u * u * p[3];
  }

  static double _bezierDeriv(List<double> p, double u) {
    final w = 1.0 - u;
    return 3.0 * w * w * (p[1] - p[0]) +
        6.0 * w * u * (p[2] - p[1]) +
        3.0 * u * u * (p[3] - p[2]);
  }

  /// Solve x(u) = t by Newton inside a shrinking bracket
  /// (docs/impl/keyframe-eval.md §2 — binding; do not substitute).
  double solveU(double t) {
    final x0 = x[0], x3 = x[3];
    if (x3 <= x0) return 0;
    var lo = 0.0, hi = 1.0;
    var u = ((t - x0) / (x3 - x0)).clamp(0.0, 1.0);
    for (var i = 0; i < 16; i++) {
      final xu = _bezier(x, u);
      if ((xu - t).abs() < 1e-12) break;
      if (xu < t) {
        lo = u;
      } else {
        hi = u;
      }
      final dxu = _bezierDeriv(x, u);
      final newton = u - (xu - t) / dxu;
      u = (dxu > 1e-12 && newton > lo && newton < hi)
          ? newton
          : 0.5 * (lo + hi);
    }
    return u;
  }

  double valueAt(double t) => _bezier(y, solveU(t));

  /// dv/dt at `t` — y′(u)/x′(u), floored so a 100%-influence handle reads as
  /// "very fast" rather than infinite (anim.rs `speed_at`).
  double speedAt(double t) {
    final u = solveU(t);
    final dx = _bezierDeriv(x, u);
    return _bezierDeriv(y, u) / (dx < 1e-12 ? 1e-12 : dx);
  }

  static double _bezierDeriv2(List<double> p, double u) =>
      6.0 * (1.0 - u) * (p[2] - 2.0 * p[1] + p[0]) +
      6.0 * u * (p[3] - 2.0 * p[2] + p[1]);

  /// How fast the speed is changing at curve parameter `u`: the derivative
  /// of y′/x′, floored the way [speedAt] is.
  double accelerationAtU(double u) {
    final dx = _bezierDeriv(x, u);
    final d = dx < 1e-12 ? 1e-12 : dx;
    return (_bezierDeriv2(y, u) * d - _bezierDeriv(y, u) * _bezierDeriv2(x, u)) /
        (d * d * d);
  }
}

/// A side's effective (speed, influence): a bezier side carries its own; an
/// automatic one is pointed by its neighbours ([autoSpeedAt], passed in as
/// [autoSpeed] by the caller that knows where the key sits); a linear (or
/// hold-in) side lies on the chord with influence ⅓ (anim.rs `side_params`
/// and `resolved_side`).
(double, double) sideParams(BridgeSideInterp side, double chordSlope,
        [double? autoSpeed]) =>
    switch (side) {
      BridgeSideInterp_Bezier(:final field0) => (
          field0.speed,
          field0.influence.clamp(1e-3, 1.0)
        ),
      BridgeSideInterp_Auto(:final field0) => (
          autoSpeed ?? chordSlope,
          field0.influence.clamp(1e-3, 1.0)
        ),
      _ => (chordSlope, 1.0 / 3.0),
    };

// ---------------------------------------------------------------------------
// Tangent modes — Auto / Clamp / Free (docs/impl/keyframe-eval.md §6).
// ---------------------------------------------------------------------------

/// The three tangent modes a keyframe side can be in — the graph strip's
/// buttons. Mirrors `lumit_core::anim::TangentMode`.
enum TangentMode { auto, clamp, free }

/// Which mode [side] is in.
TangentMode tangentModeOf(BridgeSideInterp side) => switch (side) {
      BridgeSideInterp_Auto(:final field0) =>
        field0.clamped ? TangentMode.clamp : TangentMode.auto,
      _ => TangentMode.free,
    };

/// [side] put into [mode] — the engine's `SideInterp::with_tangent_mode`,
/// ported.
///
/// The ease survives the switch: going automatic files the side's speed and
/// influence inside the automatic side, and coming back to free hands them
/// out again, so Free → Auto → Free returns the custom ease. A straight or
/// held side has no ease of its own, so it files the easy-ease pair.
BridgeSideInterp withTangentMode(BridgeSideInterp side, TangentMode mode) {
  final (speed, influence) = switch (side) {
    BridgeSideInterp_Bezier(:final field0) => (field0.speed, field0.influence),
    BridgeSideInterp_Auto(:final field0) => (field0.speed, field0.influence),
    _ => (0.0, 1.0 / 3.0),
  };
  return switch (mode) {
    TangentMode.auto => BridgeSideInterp.auto(
        BridgeAutoSide(clamped: false, speed: speed, influence: influence)),
    TangentMode.clamp => BridgeSideInterp.auto(
        BridgeAutoSide(clamped: true, speed: speed, influence: influence)),
    TangentMode.free => side is BridgeSideInterp_Auto
        ? BridgeSideInterp.bezier(
            BridgeBezierSide(speed: speed, influence: influence))
        : side,
  };
}

/// The speed an automatic side takes at `keys[index]` — anim.rs `auto_speed`,
/// ported.
///
/// In plain terms: an automatic tangent points the way the curve is already
/// going. The plain one aims straight from the key before to the key after;
/// the clamped one lies flat where the key is a peak or a trough (any tilt
/// would overshoot a neighbour) and is otherwise held to three times the
/// gentler of the two chords. An end key has no pair to aim between, so its
/// automatic tangent is flat.
double autoSpeedAt(List<BridgeKeyframe> keys, int index,
    {required bool clamped}) {
  if (index <= 0 || index + 1 >= keys.length) return 0;
  final p = keys[index - 1], k = keys[index], n = keys[index + 1];
  final tp = rationalSeconds(p.time),
      tk = rationalSeconds(k.time),
      tn = rationalSeconds(n.time);
  if (tn - tp <= 0) return 0;
  final smooth = (n.value - p.value) / (tn - tp);
  if (!clamped) return smooth;
  if (tk - tp <= 0 || tn - tk <= 0) return 0;
  final before = (k.value - p.value) / (tk - tp);
  final after = (n.value - k.value) / (tn - tk);
  if (before * after <= 0) return 0;
  final limit = 3 * (before.abs() < after.abs() ? before.abs() : after.abs());
  return smooth.clamp(-limit, limit).toDouble();
}

/// One side of `keys[index]` as every reader should see it — anim.rs
/// `resolved_side`: an automatic side becomes the bezier its neighbours
/// dictate, keeping its own influence; everything else is itself.
BridgeSideInterp resolvedSide(List<BridgeKeyframe> keys, int index,
    {required bool isOut}) {
  final side = isOut ? keys[index].interpOut : keys[index].interpIn;
  return switch (side) {
    BridgeSideInterp_Auto(:final field0) => BridgeSideInterp.bezier(
        BridgeBezierSide(
            speed: autoSpeedAt(keys, index, clamped: field0.clamped),
            influence: field0.influence)),
    _ => side,
  };
}

/// The span of a sorted key list holding `t` — the index of its first key, its
/// two keys and their times. Callers have already handled `t` outside the
/// keys, so a span always exists.
(int, BridgeKeyframe, BridgeKeyframe, double, double) _spanAt(
    List<BridgeKeyframe> keys, double t) {
  var idx = keys.length - 2;
  for (var i = 0; i + 1 < keys.length; i++) {
    if (t < rationalSeconds(keys[i + 1].time)) {
      idx = i;
      break;
    }
  }
  final a = keys[idx], b = keys[idx + 1];
  return (idx, a, b, rationalSeconds(a.time), rationalSeconds(b.time));
}

/// The cubic across the span leaving `keys[i]`, its sides read through
/// [sideParams] with an automatic one resolved against its neighbours
/// (anim.rs `span_cubic`).
CubicSpan _cubicOf(List<BridgeKeyframe> keys, int i) {
  final a = keys[i], b = keys[i + 1];
  final t1 = rationalSeconds(a.time), t2 = rationalSeconds(b.time);
  final chord = (b.value - a.value) / (t2 - t1);
  final (s1, b1) =
      sideParams(a.interpOut, chord, _autoOf(keys, i, isOut: true));
  final (s2, b2) =
      sideParams(b.interpIn, chord, _autoOf(keys, i + 1, isOut: false));
  return CubicSpan.fromAe(t1, a.value, t2, b.value,
      speedOut: s1, inflOut: b1, speedIn: s2, inflIn: b2);
}

/// The automatic speed for one side, or null where the side is not automatic
/// — so an ordinary curve pays nothing for the feature.
double? _autoOf(List<BridgeKeyframe> keys, int index, {required bool isOut}) {
  final side = isOut ? keys[index].interpOut : keys[index].interpIn;
  return side is BridgeSideInterp_Auto
      ? autoSpeedAt(keys, index, clamped: side.field0.clamped)
      : null;
}

/// Evaluate a sorted key list at `t` seconds — the engine's `evaluate`,
/// clamped past the ends, hold-out winning its span.
double evaluateKeys(List<BridgeKeyframe> keys, double t) {
  if (keys.isEmpty) return 0;
  if (t <= rationalSeconds(keys.first.time)) return keys.first.value;
  if (t >= rationalSeconds(keys.last.time)) return keys.last.value;
  final (i, a, b, t1, t2) = _spanAt(keys, t);
  final dt = t2 - t1;
  if (dt <= 0) return a.value;
  if (a.interpOut is BridgeSideInterp_Hold) return a.value;
  if (a.interpOut is BridgeSideInterp_Linear &&
      b.interpIn is BridgeSideInterp_Linear) {
    return a.value + (b.value - a.value) * ((t - t1) / dt);
  }
  return _cubicOf(keys, i).valueAt(t);
}

/// The value of a scalar at `t` seconds — a static one is itself everywhere.
double evaluateScalar(BridgeScalar scalar, double t) => switch (scalar) {
      BridgeScalar_Static(:final field0) => field0,
      BridgeScalar_Keyframed(:final field0) => evaluateKeys(field0, t),
      // An expression has no curve to draw here: only the engine can run it,
      // and this evaluator exists precisely because a paint may not cross the
      // bridge. The graph shows an expression-driven scalar as flat zero.
      BridgeScalar_Expression() => 0.0,
    };

/// dv/dt at `t` seconds — the engine's `evaluate_speed`: 0 outside the keys
/// and across a hold span, the chord on a straight span, the exact derivative
/// on a bezier one.
double evaluateKeysSpeed(List<BridgeKeyframe> keys, double t) {
  if (keys.isEmpty) return 0;
  if (t <= rationalSeconds(keys.first.time) ||
      t >= rationalSeconds(keys.last.time)) {
    return 0;
  }
  final (i, a, b, t1, t2) = _spanAt(keys, t);
  final dt = t2 - t1;
  if (dt <= 0) return 0;
  if (a.interpOut is BridgeSideInterp_Hold) return 0;
  if (a.interpOut is BridgeSideInterp_Linear &&
      b.interpIn is BridgeSideInterp_Linear) {
    return (b.value - a.value) / dt;
  }
  return _cubicOf(keys, i).speedAt(t);
}

/// d²v/dt² at `t` seconds, the acceleration graph's line: 0 wherever the speed
/// is flat (outside the keys, a hold, a straight span), the exact second
/// derivative on a bezier span.
double evaluateKeysAcceleration(List<BridgeKeyframe> keys, double t) {
  if (keys.isEmpty) return 0;
  if (t <= rationalSeconds(keys.first.time) ||
      t >= rationalSeconds(keys.last.time)) {
    return 0;
  }
  final (i, _, _, _, _) = _spanAt(keys, t);
  final cubic = _shapedSpan(keys, i);
  return cubic == null ? 0 : cubic.accelerationAtU(cubic.solveU(t));
}

/// The acceleration one side of a key reads *at the key*, where the
/// acceleration graph draws its dot. The in side ends the span before the
/// key and the out side starts the one after, so the two usually differ.
double sideAccelerationAtKey(List<BridgeKeyframe> keys, int index,
    {required bool isOut}) {
  final span = isOut ? index : index - 1;
  if (span < 0 || span + 1 >= keys.length) return 0;
  final cubic = _shapedSpan(keys, span);
  return cubic == null ? 0 : cubic.accelerationAtU(isOut ? 0 : 1);
}

/// The cubic of the span leaving `keys[i]`, or null where the span has no
/// curve to it: no length, a hold, or straight on both sides.
CubicSpan? _shapedSpan(List<BridgeKeyframe> keys, int i) {
  final a = keys[i], b = keys[i + 1];
  if (rationalSeconds(b.time) <= rationalSeconds(a.time)) return null;
  if (a.interpOut is BridgeSideInterp_Hold) return null;
  if (a.interpOut is BridgeSideInterp_Linear &&
      b.interpIn is BridgeSideInterp_Linear) {
    return null;
  }
  return _cubicOf(keys, i);
}

/// The influence that gives one side of `keys[index]` the acceleration
/// [target] at the key, keeping its speed, or null where the side has no
/// curve to shape.
///
/// At the key a side's acceleration is 2k / (3 · b² · Δt), with b its
/// influence and k fixed by the two speeds and the far side's influence. So
/// the influence reaches any acceleration with the sign of k, down to the
/// one full influence gives. Asked for the other sign, it stops at full.
double? influenceForAcceleration(List<BridgeKeyframe> keys, int index,
    {required bool isOut, required double target}) {
  final span = isOut ? index : index - 1;
  if (span < 0 || span + 1 >= keys.length) return null;
  final a = keys[span], b = keys[span + 1];
  final dt = rationalSeconds(b.time) - rationalSeconds(a.time);
  if (dt <= 0 || a.interpOut is BridgeSideInterp_Hold) return null;
  final chord = (b.value - a.value) / dt;
  final (s1, b1) =
      sideParams(a.interpOut, chord, _autoOf(keys, span, isOut: true));
  final (s2, b2) =
      sideParams(b.interpIn, chord, _autoOf(keys, span + 1, isOut: false));
  final k = isOut ? chord - s1 - b2 * (s2 - s1) : s2 - chord + b1 * (s1 - s2);
  if (k.abs() < 1e-12) return null;
  final squared = 2 * k / (3 * target * dt);
  if (!(squared > 0)) return 1.0;
  return math.sqrt(squared).clamp(1e-3, 1.0).toDouble();
}

/// The speed a key's chosen side reads *at the key* — what the speed graph's
/// dot sits at. At u=0 (and u=1) the cubic's slope is exactly the side's own
/// speed parameter, so this is closed-form: a bezier side is its speed, a
/// linear side is the chord to that neighbour, a hold side (or no neighbour)
/// is 0.
double sideSpeedAtKey(List<BridgeKeyframe> keys, int index,
    {required bool isOut}) {
  final key = keys[index];
  final neighbour = isOut
      ? (index + 1 < keys.length ? keys[index + 1] : null)
      : (index > 0 ? keys[index - 1] : null);
  if (neighbour == null) return 0;
  // The span between the two, in span order.
  final a = isOut ? key : neighbour;
  final b = isOut ? neighbour : key;
  final dt = rationalSeconds(b.time) - rationalSeconds(a.time);
  if (dt <= 0) return 0;
  if (a.interpOut is BridgeSideInterp_Hold) return 0;
  final chord = (b.value - a.value) / dt;
  final side = isOut ? key.interpOut : key.interpIn;
  return switch (side) {
    BridgeSideInterp_Bezier(:final field0) => field0.speed,
    // Automatic: the neighbours decide, not the remembered ease.
    BridgeSideInterp_Auto(:final field0) =>
      autoSpeedAt(keys, index, clamped: field0.clamped),
    BridgeSideInterp_Hold() => 0,
    _ => chord,
  };
}

/// A side's influence, defaulting to ⅓ where the side is not a bezier —
/// what a handle shows before its first drag. An automatic side keeps its own
/// reach: the neighbours point the tangent, they do not lengthen it.
double sideInfluence(BridgeSideInterp side) => switch (side) {
      BridgeSideInterp_Bezier(:final field0) =>
        field0.influence.clamp(1e-3, 1.0).toDouble(),
      BridgeSideInterp_Auto(:final field0) =>
        field0.influence.clamp(1e-3, 1.0).toDouble(),
      _ => 1.0 / 3.0,
    };

// ---------------------------------------------------------------------------
// A key's two eases as four numbers.
// ---------------------------------------------------------------------------

/// One keyframe's two eases as numbers: speed in value units per second and
/// influence as a fraction of the span, for the side the key arrives on and the
/// side it leaves by.
///
/// The same shape does two jobs. Read off a key ([keyEaseOf]) a null number is
/// a side the key has not got, which is the far side of an end key. Written
/// onto one ([keyWithEase]) a null number is one left alone, so typing a speed
/// into one side never quietly rewrites the other.
class KeyEase {
  final double? inSpeed;
  final double? inInfluence;
  final double? outSpeed;
  final double? outInfluence;

  const KeyEase({
    this.inSpeed,
    this.inInfluence,
    this.outSpeed,
    this.outInfluence,
  });

  bool get hasIn => inSpeed != null || inInfluence != null;
  bool get hasOut => outSpeed != null || outInfluence != null;
  bool get isEmpty => !hasIn && !hasOut;

  /// This ease with [other]'s numbers laid over it, where [other] has any.
  KeyEase merge(KeyEase other) => KeyEase(
        inSpeed: other.inSpeed ?? inSpeed,
        inInfluence: other.inInfluence ?? inInfluence,
        outSpeed: other.outSpeed ?? outSpeed,
        outInfluence: other.outInfluence ?? outInfluence,
      );

  /// Only the side named, the other side dropped.
  KeyEase side({required bool isOut}) => isOut
      ? KeyEase(outSpeed: outSpeed, outInfluence: outInfluence)
      : KeyEase(inSpeed: inSpeed, inInfluence: inInfluence);

  @override
  bool operator ==(Object other) =>
      other is KeyEase &&
      other.inSpeed == inSpeed &&
      other.inInfluence == inInfluence &&
      other.outSpeed == outSpeed &&
      other.outInfluence == outInfluence;

  @override
  int get hashCode => Object.hash(inSpeed, inInfluence, outSpeed, outInfluence);

  @override
  String toString() =>
      'KeyEase(in: $inSpeed @ $inInfluence, out: $outSpeed @ $outInfluence)';
}

/// The eases `keys[index]` reads at: what its handles show, with an automatic
/// side resolved against its neighbours. An end key has one side only.
KeyEase keyEaseOf(List<BridgeKeyframe> keys, int index) {
  final key = keys[index];
  final hasIn = index > 0;
  final hasOut = index + 1 < keys.length;
  return KeyEase(
    inSpeed: hasIn ? sideSpeedAtKey(keys, index, isOut: false) : null,
    inInfluence: hasIn ? sideInfluence(key.interpIn) : null,
    outSpeed: hasOut ? sideSpeedAtKey(keys, index, isOut: true) : null,
    outInfluence: hasOut ? sideInfluence(key.interpOut) : null,
  );
}

/// `keys[index]` with the numbers in [ease] written onto it.
///
/// A side given either number becomes a bezier at that speed and that
/// influence, taking whichever of the two was not given from what the side
/// reads now, so a straight side given a speed keeps the reach its handle
/// already showed. A bezier is a Free side, which is what typing a number
/// means: the hand has overruled the neighbours. A side given nothing is left
/// exactly as it was.
BridgeKeyframe keyWithEase(List<BridgeKeyframe> keys, int index, KeyEase ease) {
  final key = keys[index];
  BridgeSideInterp side(bool isOut, double? speed, double? influence) {
    final was = isOut ? key.interpOut : key.interpIn;
    if (speed == null && influence == null) return was;
    return BridgeSideInterp.bezier(BridgeBezierSide(
      speed: speed ?? sideSpeedAtKey(keys, index, isOut: isOut),
      influence: (influence ?? sideInfluence(was))
          .clamp(minTangentReach, 1.0)
          .toDouble(),
    ));
  }

  return BridgeKeyframe(
    time: key.time,
    value: key.value,
    interpIn: side(false, ease.inSpeed, ease.inInfluence),
    interpOut: side(true, ease.outSpeed, ease.outInfluence),
  );
}

/// [keys] with [keyWithEase] applied to the one at [index].
List<BridgeKeyframe> keysWithEase(
        List<BridgeKeyframe> keys, int index, KeyEase ease) =>
    [
      for (var i = 0; i < keys.length; i++)
        if (i == index) keyWithEase(keys, i, ease) else keys[i],
    ];

// ---------------------------------------------------------------------------
// Tangent-handle geometry (value lens).
// ---------------------------------------------------------------------------

/// Where a key's tangent handle endpoint sits, in (seconds, value) — the
/// bezier control point itself: reach `influence·Δt` toward the neighbour,
/// rise `speed` per second along it.
({double time, double value}) handleEndpoint({
  required double keyTime,
  required double keyValue,
  required double neighbourTime,
  required bool isOut,
  required double speed,
  required double influence,
}) {
  final dt = (neighbourTime - keyTime).abs();
  final reach = influence * dt;
  return (
    time: isOut ? keyTime + reach : keyTime - reach,
    value: isOut ? keyValue + speed * reach : keyValue - speed * reach,
  );
}

/// The least of its span a tangent may reach across, so it is never quite
/// vertical.
///
/// A *perfectly* vertical tangent covers no time at all, and that is the one
/// shape this geometry cannot come back from: with zero reach there is no
/// speed that describes it, so the length it was drawn at is unrecoverable and
/// the handle would return somewhere else. Held a hair off vertical everything
/// stays reversible — a thousandth of the gap is far under a pixel at any
/// sane zoom, and no ease anyone shapes can tell the difference.
const double minTangentReach = 1e-3;

/// A dragged handle endpoint read back into (speed, influence) — the inverse
/// of [handleEndpoint], with the reach clamped inside the span so the handle
/// lands under the cursor, the curve stays x-monotone, and the tangent never
/// stands exactly upright ([minTangentReach]).
({double speed, double influence}) handleFromDrag({
  required double keyTime,
  required double keyValue,
  required double neighbourTime,
  required bool isOut,
  required double dragTime,
  required double dragValue,
}) {
  final dt = (neighbourTime - keyTime).abs();
  if (dt <= 1e-9) return (speed: 0, influence: 1 / 3);
  final reach = (isOut ? dragTime - keyTime : keyTime - dragTime)
      .clamp(dt * minTangentReach, dt);
  final influence = (reach / dt).clamp(minTangentReach, 1.0).toDouble();
  final speed = (isOut ? dragValue - keyValue : keyValue - dragValue) / reach;
  return (speed: speed, influence: influence);
}

// ---------------------------------------------------------------------------
// Vertical framing (auto-fit).
// ---------------------------------------------------------------------------

/// The (low, high) a set of curves needs vertically: every key's value, every
/// bezier handle endpoint (a steep handle pokes past the curve), and the
/// curve's own samples (a bezier overshoots its keys), padded 15%. `values`
/// carries each channel's keys; a static channel contributes its one value.
(double, double) fitValueRange(
  List<List<BridgeKeyframe>> channels,
  List<double> staticValues, {
  double? timeLo,
  double? timeHi,
}) {
  var lo = double.infinity, hi = -double.infinity;
  void grow(double v) {
    if (v < lo) lo = v;
    if (v > hi) hi = v;
  }

  for (final v in staticValues) {
    grow(v);
  }
  for (final keys in channels) {
    for (var i = 0; i < keys.length; i++) {
      final k = keys[i];
      grow(k.value);
      final t = rationalSeconds(k.time);
      for (final isOut in const [true, false]) {
        final nb = isOut
            ? (i + 1 < keys.length ? keys[i + 1] : null)
            : (i > 0 ? keys[i - 1] : null);
        if (nb == null) continue;
        final side = isOut ? k.interpOut : k.interpIn;
        if (side is! BridgeSideInterp_Bezier &&
            side is! BridgeSideInterp_Auto) {
          continue;
        }
        final e = handleEndpoint(
          keyTime: t,
          keyValue: k.value,
          neighbourTime: rationalSeconds(nb.time),
          isOut: isOut,
          speed: sideSpeedAtKey(keys, i, isOut: isOut),
          influence: sideInfluence(side),
        );
        grow(e.value);
      }
    }
    if (keys.length >= 2) {
      final t0 = timeLo ?? rationalSeconds(keys.first.time);
      final t1 = timeHi ?? rationalSeconds(keys.last.time);
      for (var s = 0; s <= 64; s++) {
        grow(evaluateKeys(keys, t0 + (t1 - t0) * s / 64));
      }
    }
  }
  if (lo > hi) return (0, 1);
  final pad =
      ((hi - lo).abs() < 1e-9 ? (lo.abs() < 1 ? 1.0 : lo.abs()) : hi - lo) *
          0.15;
  return (lo - pad, hi + pad);
}

/// The same framing for the speed lens: every span side's speed at its keys,
/// plus dense samples of the derivative (a bezier span's speed peaks between
/// keys), padded 15% and always including 0 so the axis has its floor.
(double, double) fitSpeedRange(List<List<BridgeKeyframe>> channels) =>
    _fitRate(channels, sideSpeedAtKey, evaluateKeysSpeed);

/// The same again for the acceleration lens.
(double, double) fitAccelerationRange(List<List<BridgeKeyframe>> channels) =>
    _fitRate(channels, sideAccelerationAtKey, evaluateKeysAcceleration);

(double, double) _fitRate(
  List<List<BridgeKeyframe>> channels,
  double Function(List<BridgeKeyframe>, int, {required bool isOut}) atKey,
  double Function(List<BridgeKeyframe>, double) along,
) {
  var lo = 0.0, hi = 0.0;
  void grow(double v) {
    if (v < lo) lo = v;
    if (v > hi) hi = v;
  }

  for (final keys in channels) {
    for (var i = 0; i < keys.length; i++) {
      grow(atKey(keys, i, isOut: true));
      grow(atKey(keys, i, isOut: false));
    }
    if (keys.length >= 2) {
      final t0 = rationalSeconds(keys.first.time);
      final t1 = rationalSeconds(keys.last.time);
      for (var s = 0; s <= 64; s++) {
        grow(along(keys, t0 + (t1 - t0) * s / 64));
      }
    }
  }
  final pad = ((hi - lo).abs() < 1e-9 ? 1.0 : hi - lo) * 0.15;
  return (lo - pad, hi + pad);
}

// ---------------------------------------------------------------------------
// The keyframe clipboard text (docs/07 §5.3).
// ---------------------------------------------------------------------------

/// One property's worth of clipboard rows: the property line (tab-joined, e.g.
/// `Transform<TAB>Position`), its value column headings, and one row per frame
/// carrying that property's values — and each value's easing — in column order.
class LumitClipGroup {
  final List<String> property;
  final List<String> columns;
  final List<LumitClipRow> rows;
  const LumitClipGroup({
    required this.property,
    required this.columns,
    required this.rows,
  });
}

/// One frame's worth of a property: its values in column order, and each
/// column's `(in, out)` easing. [eases] is either empty or as long as [values].
class LumitClipRow {
  final double frame;
  final List<double> values;
  final List<(BridgeSideInterp, BridgeSideInterp)> eases;
  const LumitClipRow({
    required this.frame,
    required this.values,
    this.eases = const [],
  });
}

/// How a side's easing is written in the clipboard text: `linear`, `hold`,
/// `bezier(speed,influence)`, or — for an automatic side —
/// `auto(speed,influence)` / `clamp(speed,influence)`, carrying the ease it is
/// remembering so a copied key pastes in the mode it was in.
String easeToText(BridgeSideInterp side) => switch (side) {
      BridgeSideInterp_Hold() => 'hold',
      BridgeSideInterp_Bezier(:final field0) =>
        'bezier(${_number(field0.speed)},${_number(field0.influence)})',
      BridgeSideInterp_Auto(:final field0) =>
        '${field0.clamped ? 'clamp' : 'auto'}'
            '(${_number(field0.speed)},${_number(field0.influence)})',
      _ => 'linear',
    };

/// The easing a clipboard cell names, defaulting to linear for anything this
/// build does not recognise — including a table from another editor, which
/// carries values and no easing at all.
BridgeSideInterp easeFromText(String text) {
  final cell = text.trim().toLowerCase();
  if (cell == 'hold') return const BridgeSideInterp.hold();
  for (final name in const ['bezier', 'auto', 'clamp']) {
    if (!cell.startsWith('$name(') || !cell.endsWith(')')) continue;
    final parts = cell.substring(name.length + 1, cell.length - 1).split(',');
    if (parts.length != 2) break;
    final speed = double.tryParse(parts[0]);
    final influence = double.tryParse(parts[1]);
    if (speed == null || influence == null) break;
    final reach = influence.clamp(1e-3, 1.0).toDouble();
    return name == 'bezier'
        ? BridgeSideInterp.bezier(
            BridgeBezierSide(speed: speed, influence: reach))
        : BridgeSideInterp.auto(BridgeAutoSide(
            clamped: name == 'clamp', speed: speed, influence: reach));
  }
  return const BridgeSideInterp.linear();
}

/// The keyframe clipboard text for [groups].
///
/// A tab-separated table under a named header, in the shape the
/// motion-graphics world already expects of copied keyframes — so a copied
/// ramp can be read by a script, dropped in a spreadsheet, or ported to
/// another tool — extended with **two easing columns per value** so a bezier
/// survives the round trip instead of flattening to linear. The easing columns
/// come last, after every value, so a reader that does not know them simply
/// stops at the values it does.
String lumitClipboardText({
  required String version,
  required double fps,
  required int width,
  required int height,
  required List<LumitClipGroup> groups,
}) {
  final b = StringBuffer()
    ..writeln('Lumit $version Keyframe Data')
    ..writeln()
    ..writeln('\tUnits Per Second\t${_number(fps)}')
    ..writeln('\tSource Width\t$width')
    ..writeln('\tSource Height\t$height')
    ..writeln('\tSource Pixel Aspect Ratio\t1')
    ..writeln('\tComp Pixel Aspect Ratio\t1')
    ..writeln();
  for (final group in groups) {
    final eased = group.rows.any((r) => r.eases.isNotEmpty);
    final heading = [
      ...group.columns,
      if (eased)
        for (final column in group.columns) ...[
          '$column$easeInSuffix',
          '$column$easeOutSuffix',
        ],
    ];
    b
      ..writeln(group.property.join('\t'))
      ..writeln('\tFrame\t${heading.join('\t')}\t');
    for (final row in group.rows) {
      const linear = BridgeSideInterp.linear();
      final cells = [
        ...row.values.map(_number),
        if (eased)
          for (var i = 0; i < row.values.length; i++) ...[
            easeToText(i < row.eases.length ? row.eases[i].$1 : linear),
            easeToText(i < row.eases.length ? row.eases[i].$2 : linear),
          ],
      ];
      b.writeln('\t${_number(row.frame)}\t${cells.join('\t')}\t');
    }
    b.writeln();
  }
  b.writeln('End of Keyframe Data');
  return b.toString();
}

/// What marks the two easing columns a value column carries.
const String easeInSuffix = ' Ease In';
const String easeOutSuffix = ' Ease Out';

/// Numbers written plainly: integers bare, fractions trimmed.
String _number(double v) {
  if (v == v.roundToDouble()) return v.round().toString();
  var s = v.toStringAsFixed(6);
  while (s.endsWith('0')) {
    s = s.substring(0, s.length - 1);
  }
  if (s.endsWith('.')) s = s.substring(0, s.length - 1);
  return s;
}

/// Parsed keyframe clipboard text, or null when the text is not that.
///
/// Tolerant on purpose: only the header line, the rate and the group tables
/// matter, the easing columns are optional, and anything else is skipped — so
/// a keyframe table from another editor still pastes, as linear keys.
({double fps, List<LumitClipGroup> groups})? parseClipboardText(String text) {
  final lines = text.split(RegExp(r'\r?\n'));
  if (lines.isEmpty || !lines.first.contains('Keyframe Data')) return null;
  var fps = 0.0;
  final groups = <LumitClipGroup>[];
  List<String>? property;
  List<String>? columns;
  List<LumitClipRow>? rows;
  var valueCount = 0;
  var hasEases = false;

  void closeGroup() {
    if (property != null && columns != null && rows != null) {
      groups.add(
          LumitClipGroup(property: property!, columns: columns!, rows: rows!));
    }
    property = null;
    columns = null;
    rows = null;
  }

  for (final line in lines.skip(1)) {
    if (line.trim().isEmpty || line.startsWith('End of Keyframe Data')) {
      closeGroup();
      continue;
    }
    final cells = line.split('\t');
    if (!line.startsWith('\t')) {
      // A property line: `Transform<TAB>Position`, `Effects<TAB>…`.
      closeGroup();
      property = [for (final c in cells) c.trim()];
      continue;
    }
    final body = [for (final c in cells.skip(1)) c.trim()];
    if (body.isEmpty) continue;
    if (body.first == 'Units Per Second') {
      fps = double.tryParse(body.elementAtOrNull(1) ?? '') ?? 0;
    } else if (body.first == 'Frame') {
      final heading = [
        for (final c in body.skip(1))
          if (c.isNotEmpty) c,
      ];
      // Every value column carries two easing columns after all the values, so
      // an eased table is exactly three times as wide as it has values — and
      // the first column past the values says so in its name.
      hasEases = heading.length >= 3 &&
          heading.length % 3 == 0 &&
          heading[heading.length ~/ 3].endsWith(easeInSuffix);
      valueCount = hasEases ? heading.length ~/ 3 : heading.length;
      columns = heading.take(valueCount).toList();
      rows = [];
    } else if (rows != null) {
      final frame = double.tryParse(body.first);
      if (frame == null) continue;
      final cells = body.skip(1).toList();
      // A table whose value columns are unnamed — which foreign keyframe text
      // often is — says nothing about how many there are, so every number on
      // the row counts as one.
      final values = valueCount == 0
          ? [
              for (final c in cells)
                if (double.tryParse(c) != null) double.parse(c),
            ]
          : [
              for (var i = 0; i < valueCount && i < cells.length; i++)
                double.tryParse(cells[i]) ?? 0,
            ];
      final eases = <(BridgeSideInterp, BridgeSideInterp)>[];
      if (hasEases) {
        for (var i = 0; i < values.length; i++) {
          final at = valueCount + i * 2;
          if (at + 1 >= cells.length) break;
          eases.add((easeFromText(cells[at]), easeFromText(cells[at + 1])));
        }
      }
      rows!.add(LumitClipRow(frame: frame, values: values, eases: eases));
    }
  }
  closeGroup();
  if (fps <= 0 || groups.isEmpty) return null;
  return (fps: fps, groups: groups);
}

// ---------------------------------------------------------------------------
// The Vegas speed envelope.
// ---------------------------------------------------------------------------

/// The vertical range a Retime channel's envelope opens at, in per cent.
///
/// Headroom over normal playback, and enough below zero to show that dragging
/// a point down there runs the clip backwards. The room above 100% is the
/// point of the top figure: at exactly 100 the flat line every un-retimed clip
/// draws sat on the very top edge of the graph, with nowhere to go but down —
/// which reads as a ceiling rather than as the ordinary speed it is. It only
/// ever grows: a curve reaching past either end reframes the axis.
const (double, double) envelopeDefaultRange = (-25.0, 125.0);

/// The influence that makes a cubic side lie on its chord — the polynomial
/// subclass. The envelope authors every side at this influence, which
/// is what makes its straight lines exactly straight (see [envelopeToKeys]).
const double _chordInfluence = 1 / 3;

/// The shape the speed takes between two envelope points, by Vegas's names.
/// Fast changes most at the start, Slow most at the end, Smooth eases at
/// both ends, Sharp at neither, and Hold keeps the first speed to the next
/// point.
enum SpeedCurve { linear, fast, slow, smooth, sharp, hold }

/// The influence each curve is written at, which is also how it is read back.
/// Nothing else in a key can carry the choice, so each curve gets its own
/// value. Fast, Slow and Hold sit just beside ⅓, where their speed is all but
/// exactly the parabola (or flat line) the name promises.
const Map<SpeedCurve, double> _curveInfluence = {
  SpeedCurve.linear: _chordInfluence,
  SpeedCurve.fast: 0.34,
  SpeedCurve.slow: 0.35,
  SpeedCurve.hold: 0.36,
  SpeedCurve.smooth: 0.6,
  SpeedCurve.sharp: 0.12,
};

/// The curve a span is written in, read from the side leaving its first key.
/// A side shaped some other way, in the Time lens, reads as Linear.
SpeedCurve speedCurveOf(BridgeSideInterp side) {
  if (side is! BridgeSideInterp_Bezier) return SpeedCurve.linear;
  for (final MapEntry(:key, :value) in _curveInfluence.entries) {
    if ((side.field0.influence - value).abs() < 0.002) return key;
  }
  return SpeedCurve.linear;
}

/// One curve per span of [keys].
List<SpeedCurve> envelopeCurves(List<BridgeKeyframe> keys) => [
      for (var i = 0; i + 1 < keys.length; i++) speedCurveOf(keys[i].interpOut),
    ];

/// How much of the step from one speed to the next a curve's span plays at,
/// on average: the area under it. Fast is 2u − u², Slow u², Hold none of it.
double _stepShare(SpeedCurve curve) => switch (curve) {
      SpeedCurve.fast => 2 / 3,
      SpeedCurve.slow => 1 / 3,
      SpeedCurve.hold => 0,
      _ => 0.5,
    };

/// The envelope's points for [keys]: the playback speed at each key, in per
/// cent, where 100 is source rate and a negative value runs backwards.
///
/// A key's two sides carry a speed each, and an envelope point is a key whose
/// two agree ([envelopeToKeys] writes them that way). A channel shaped in the
/// Time lens instead can disagree at a key; the point then reads the side
/// facing *into* the clip — the outgoing speed, or the incoming one at the
/// last key — because that is the speed the span it governs is played at.
///
/// The last point after a Hold span arrives at the held speed, so its own
/// is kept on its outgoing side, which plays nothing.
List<double> envelopeSpeeds(List<BridgeKeyframe> keys) => [
      for (var i = 0; i < keys.length; i++)
        switch (keys[i].interpOut) {
          BridgeSideInterp_Bezier(:final field0) when i == keys.length - 1 =>
            field0.speed,
          _ => sideSpeedAtKey(keys, i, isOut: i < keys.length - 1),
        } *
            100,
    ];

/// Keys carrying [speeds] (per cent, one per key), with the source positions
/// re-integrated from them and the **first key pinned**.
///
/// This is the Vegas edit: change a speed and the frames after it change,
/// while every keyframe *time* and the layer's own box stay exactly where they
/// are. The first key is what "pinned" means — a clip's first frame is its own
/// trim-in whatever its speed, so re-speeding never moves where it starts.
///
/// **Why the trapezoid is exact rather than an approximation.** Between two
/// points a Linear span draws a straight line, so the source advanced across
/// it is the area under that line: the average of the two speeds times the
/// span. Setting a cubic's value change to exactly that, with its endpoint
/// slopes at the two speeds and influence ⅓, makes the cubic's own derivative
/// come out *exactly* that straight line — the u² term cancels. So the
/// envelope is not a simplified view of the curve underneath; it is the same
/// curve, read the other way round. (Substitute Δ/d = (m₀+m₁)/2 into a cubic
/// Hermite's derivative and the quadratic coefficient is zero.) Fast and Slow
/// move the area to two thirds and one third of the step, which leaves the
/// parabola leaning the named way; Smooth and Sharp keep the half and lean
/// on the influence instead.
///
/// [curves], one per span, defaults to the curves [keys] are written in.
List<BridgeKeyframe> envelopeToKeys(
    List<BridgeKeyframe> keys, List<double> speeds,
    {List<SpeedCurve>? curves}) {
  if (keys.isEmpty || speeds.length != keys.length) return keys;
  final shapes = curves ?? envelopeCurves(keys);
  if (shapes.length != keys.length - 1) return keys;
  // Every key's new source position first, so each side can be compared
  // against the chord of the span it governs.
  final values = <double>[keys.first.value];
  for (var i = 1; i < keys.length; i++) {
    final dt =
        rationalSeconds(keys[i].time) - rationalSeconds(keys[i - 1].time);
    final from = speeds[i - 1], to = speeds[i];
    // Non-positive only for keys sharing a time, which the editing ops
    // forbid; treated as no advance rather than as a reason to fail.
    values.add(dt > 0
        ? values[i - 1] +
            (from + (to - from) * _stepShare(shapes[i - 1])) / 100 * dt
        : values[i - 1]);
  }

  /// The side governing span `s` (keys `s` and `s + 1`), from its start when
  /// [isOut]. **A Linear span is linear when its speed is the chord** — the
  /// two are then the same curve, and writing the bezier form anyway would
  /// change how the key *looks* (a diamond becomes a circle) for no change in
  /// what it does. Dragging one point of a flat envelope used to re-shape
  /// every key on the channel that way.
  BridgeSideInterp side(int s, {required bool isOut}) {
    final curve = shapes[s];
    // Hold arrives at the next point still at the speed it left with.
    final at = isOut || curve == SpeedCurve.hold ? s : s + 1;
    final speed = speeds[at] / 100;
    if (curve == SpeedCurve.linear) {
      final dt =
          rationalSeconds(keys[s + 1].time) - rationalSeconds(keys[s].time);
      final chord = dt > 0 ? (values[s + 1] - values[s]) / dt : 0.0;
      if ((speed - chord).abs() < 1e-9) return const BridgeSideInterp.linear();
    }
    return BridgeSideInterp.bezier(
      BridgeBezierSide(speed: speed, influence: _curveInfluence[curve]!),
    );
  }

  return [
    for (var i = 0; i < keys.length; i++)
      BridgeKeyframe(
        time: keys[i].time,
        value: values[i],
        // The first key has no span before it and the last none after it;
        // those sides govern nothing, so they stay as plain as possible.
        interpIn: i == 0
            ? const BridgeSideInterp.linear()
            : side(i - 1, isOut: false),
        interpOut: i < keys.length - 1
            ? side(i, isOut: true)
            : i > 0 && shapes[i - 1] == SpeedCurve.hold
                ? BridgeSideInterp.bezier(BridgeBezierSide(
                    speed: speeds[i] / 100, influence: _chordInfluence))
                : const BridgeSideInterp.linear(),
      ),
  ];
}

/// [keys] with span [span] played in [curve].
List<BridgeKeyframe> setEnvelopeCurve(
    List<BridgeKeyframe> keys, int span, SpeedCurve curve) {
  if (span < 0 || span + 1 >= keys.length) return keys;
  final curves = envelopeCurves(keys)..[span] = curve;
  return envelopeToKeys(keys, envelopeSpeeds(keys), curves: curves);
}

/// [keys] with the envelope point at [index] moved to [time], keeping the
/// speed it had.
///
/// **The values are re-integrated, not carried over.** A key's stored tangent
/// is a speed; the span's *chord* is its average. Move a key in time and the
/// chord changes while the tangent does not, so a span that was straight stops
/// being straight — the curve bulges and the graph starts describing playback
/// that is not what the points say. Re-running the integration through the
/// same speeds puts every span back on its own straight line, which is what a
/// later speed drag was silently doing and why the fault appeared to fix
/// itself the second time you touched a point.
List<BridgeKeyframe> moveEnvelopePoint(
    List<BridgeKeyframe> keys, int index, BridgeRational time) {
  if (index < 0 || index >= keys.length) return keys;
  final speeds = envelopeSpeeds(keys);
  final moved = [
    for (var i = 0; i < keys.length; i++)
      if (i == index)
        BridgeKeyframe(
          time: time,
          value: keys[i].value,
          interpIn: keys[i].interpIn,
          interpOut: keys[i].interpOut,
        )
      else
        keys[i],
  ];
  return envelopeToKeys(moved, speeds, curves: envelopeCurves(keys));
}

/// [keys] with the envelope point at [index] moved to [percent].
List<BridgeKeyframe> setEnvelopeSpeed(
    List<BridgeKeyframe> keys, int index, double percent) {
  if (index < 0 || index >= keys.length) return keys;
  final speeds = envelopeSpeeds(keys);
  speeds[index] = percent;
  return envelopeToKeys(keys, speeds);
}

/// The framing for an envelope: [envelopeDefaultRange], grown to hold every
/// point. Unlike the other lenses this has a floor and a ceiling to start
/// from, because "100% is normal" is a fact about the axis rather than
/// something to discover from the data.
(double, double) fitEnvelopeRange(List<List<BridgeKeyframe>> channels) {
  var (lo, hi) = envelopeDefaultRange;
  for (final keys in channels) {
    for (final v in envelopeSpeeds(keys)) {
      if (v < lo) lo = v;
      if (v > hi) hi = v;
    }
  }
  // Padded only where the data pushed past the default, so an untouched
  // channel opens at exactly the documented range.
  final (dlo, dhi) = envelopeDefaultRange;
  final pad = (hi - lo) * 0.08;
  return (lo < dlo ? lo - pad : dlo, hi > dhi ? hi + pad : dhi);
}

/// The speed, in per cent, at which footage shows every one of its frames
/// once: the comp's rate over the footage's, so 300 fps footage in a 24 fps
/// comp is 8. Below it frames repeat, above it frames are skipped.
///
/// Null unless the footage is the faster of the two. Rates half a per cent
/// apart count as the same, so 24 against 23.976 draws no guide beside 100%.
double? everyFrameSpeed(double compFps, double footageFps) =>
    compFps > 0 && footageFps > compFps * 1.005
        ? compFps / footageFps * 100
        : null;

/// How near the every-frame guide a dragged point has to come to land on it,
/// in pixels.
const double envelopeSnapReach = 3;

/// [speed] taken onto [guide] when it is within [reach] per cent of it.
double snapEnvelopeSpeed(double speed, double? guide, double reach) =>
    guide != null && (speed - guide).abs() <= reach ? guide : speed;

/// A speed as a point's drag readout writes it, without the sign: whole per
/// cent, or one decimal place while the drag is [fine]. Only the reading is
/// rounded, never the speed the drag stores.
String speedText(double percent, {required bool fine}) =>
    fine ? percent.toStringAsFixed(1) : '${percent.round()}';

/// The every-frame guide's own number: two places, with the noughts trimmed,
/// so 8 reads 8 and 23.976 in 300 reads 7.99.
String guideSpeedText(double percent) =>
    percent.toStringAsFixed(2).replaceFirst(RegExp(r'\.?0+$'), '');

// ---------------------------------------------------------------------------
// Grid ruling.
// ---------------------------------------------------------------------------

/// The power of ten at or just under [v] (0.03 -> 0.01, 30 -> 10).
///
/// **Anything that is not a finite positive number answers 1**, and that
/// refusal is the whole reason this is a named function rather than four lines
/// in a painter. The second loop asks `p * 10 <= v`, which never stops being
/// true once [v] is infinity: `p` climbs to 1e308, becomes infinity itself, and
/// infinity is not greater than infinity. On the interface's own thread that is
/// not a wrong number on a grid, it is an application that never draws again.
double powerOfTenUnder(double v) {
  if (!v.isFinite || v <= 0) return 1;
  var p = 1.0;
  while (p > v) {
    p /= 10;
  }
  while (p * 10 <= v) {
    p *= 10;
  }
  return p;
}

/// The values a graph pane rules its grid at between [lo] and [hi]: a nice
/// step whose lines sit at least ~36 px apart down [height].
///
/// Empty when the range is not a range at all — **including a range with an
/// infinity or a NaN at either end**. That bound used to reach
/// [powerOfTenUnder] through a `clamp(1e-12, double.infinity)` that reads like
/// a guard and is the opposite of one: Dart's `num.clamp` orders NaN above its
/// upper limit, so a NaN step came back as exactly the infinity that hangs.
/// The painter's own `isFinite` check sat below the call and never ran.
List<double> gridValues(double lo, double hi, double height) {
  if (!lo.isFinite || !hi.isFinite) return const [];
  final span = (hi - lo).abs() < 1e-12 ? 1.0 : hi - lo;
  final rawStep = span / (height / 36).clamp(1, 12);
  final magnitude = powerOfTenUnder(rawStep.abs());
  var step = magnitude;
  for (final m in const [1.0, 2.0, 5.0, 10.0]) {
    if (magnitude * m >= rawStep) {
      step = magnitude * m;
      break;
    }
  }
  if (!step.isFinite || step <= 0) return const [];
  final out = <double>[];
  for (var v = (lo / step).ceilToDouble() * step; v <= hi; v += step) {
    out.add(v);
    // A range and a step that disagree by a rounding error would otherwise
    // rule a line per pixel for ever.
    if (out.length > 64) break;
  }
  return out;
}

/// A grid line's number, to as many places as [step] needs. One place for
/// every step put the same number on two lines once the step was under 0.1.
String gridLabel(double v, double step) {
  if (v.abs() >= 100) return v.round().toString();
  var places = 1;
  for (var s = step.abs(); s > 0 && s < 0.0999 && places < 6; s *= 10) {
    places++;
  }
  // Whole numbers stay plain, and the steps are summed so they land a hair off.
  final text = v.toStringAsFixed(places);
  final shown = double.parse(text);
  return shown == shown.roundToDouble() ? shown.round().toString() : text;
}

/// **What interpolation a brand-new keyframe is born with** (M19).
///
/// Not a fixed default: it takes after the keys it lands between. The half
/// facing backwards takes the kind of the previous key's *outgoing* side, and
/// the half facing forwards the kind of the next key's *incoming* side, so a
/// key planted in the middle of a held run holds, one planted between two eased
/// keys eases, and one planted between a hold and an ease does both — each side
/// matching what it faces. With a neighbour on one side only, both halves take
/// that one; with no neighbours at all, linear, which is what a first key has
/// always been.
///
/// **The kind, not the shape.** A bezier side's speed is in value units per
/// second and its influence is a reach in time — both belong to the key that
/// stores them, so copying them onto a key somewhere else would be copying a
/// curve nobody drew. A new bezier side is therefore the easy ease every fresh
/// bezier is (speed 0, influence a third), and an Auto side is carried over as
/// it stands, since an Auto tangent works itself out from its neighbours
/// anyway.
BridgeSideInterp inheritedSide(BridgeSideInterp? facing) => switch (facing) {
      BridgeSideInterp_Hold() => const BridgeSideInterp.hold(),
      BridgeSideInterp_Bezier() =>
        BridgeSideInterp.bezier(BridgeBezierSide(speed: 0, influence: 1 / 3)),
      BridgeSideInterp_Auto() => facing,
      _ => const BridgeSideInterp.linear(),
    };

/// A new keyframe at [seconds] carrying [value], taking its interpolation from
/// the keys around it (M19). [keys] need not be sorted; a key already at that
/// moment is ignored, since planting on one is an edit and not an insertion.
BridgeKeyframe keyframeAmong(
  List<BridgeKeyframe> keys,
  BridgeRational time,
  double value,
) {
  final seconds = rationalSeconds(time);
  BridgeKeyframe? before;
  BridgeKeyframe? after;
  for (final key in keys) {
    final at = rationalSeconds(key.time);
    if (at < seconds && (before == null || at > rationalSeconds(before.time))) {
      before = key;
    }
    if (at > seconds && (after == null || at < rationalSeconds(after.time))) {
      after = key;
    }
  }
  return BridgeKeyframe(
    time: time,
    value: value,
    interpIn: inheritedSide(before?.interpOut ?? after?.interpIn),
    interpOut: inheritedSide(after?.interpIn ?? before?.interpOut),
  );
}
