// The Retime speed-lens maths, all pure so it unit-tests without a widget tree.
// Ported from `crates/lumit-core/src/retime.rs` (the `Ease` shapes, the rate
// speed profile v0 + (v1−v0)·e(u), and the Map-segment derivative
// y′(u)/x′(u)) and from `crates/lumit-ui/src/shell/graph.rs::graph_plot_retime`
// (the speed-view sampling and the boundary handling), adapted to work in comp
// *frames* on the x axis so the curve shares the timeline ruler's zoom mapping
// rather than a whole-width t/duration scale.
//
// In plain terms: this file knows the shape of each speed ramp, how to turn a
// Retime store into a list of (frame, speed-%) points to draw, which boundary a
// pointer is grabbing, and how far a dragged boundary may travel before it hits
// its neighbours.

import 'dart:math' as math;

import '../../bridge/bridge.dart';

/// The five Vegas ease profiles (docs/04-RETIMING.md §4.1), matching
/// `lumit_core::retime::Ease`.
enum GraphEase { linear, slow, fast, smooth, sharp }

/// The ease named by a segment's serde variant (`Linear`/`Slow`/`Fast`/
/// `Smooth`/`Sharp`), defaulting to [GraphEase.linear] for anything else.
GraphEase easeFromName(String? name) => switch (name) {
      'Slow' => GraphEase.slow,
      'Fast' => GraphEase.fast,
      'Smooth' => GraphEase.smooth,
      'Sharp' => GraphEase.sharp,
      _ => GraphEase.linear,
    };

/// The five ramp presets as the labels the header draws.
const List<String> presetLabels = ['Lin', 'Slow', 'Fast', 'Smth', 'Shrp'];

/// The preset-row label for an ease (the exact string the `setSegmentPreset`
/// op takes: `Lin`/`Slow`/`Fast`/`Smth`/`Shrp`).
String presetLabelFor(GraphEase e) => switch (e) {
      GraphEase.linear => 'Lin',
      GraphEase.slow => 'Slow',
      GraphEase.fast => 'Fast',
      GraphEase.smooth => 'Smth',
      GraphEase.sharp => 'Shrp',
    };

/// e(u), the speed-profile shape itself (0 at the segment start, 1 at the end)
/// — `lumit_core::retime::Ease::small_e`, ported line-for-line.
double smallE(GraphEase ease, double u) {
  switch (ease) {
    case GraphEase.linear:
      return u;
    case GraphEase.slow:
      return u * u;
    case GraphEase.fast:
      return 2.0 * u - u * u;
    case GraphEase.smooth:
      if (u <= 0.5) return 2.0 * u * u;
      final w = 1.0 - u;
      return 1.0 - 2.0 * w * w;
    case GraphEase.sharp:
      if (u <= 0.5) return 2.0 * u - 2.0 * u * u;
      return 2.0 * u * u - 2.0 * u + 1.0;
  }
}

/// E(u), the integral of e(u) (0 at the segment start) — `retime.rs::big_e`,
/// ported line-for-line. Used to map local time → source position (`evaluate`).
double bigE(GraphEase ease, double u) {
  switch (ease) {
    case GraphEase.linear:
      return u * u / 2.0;
    case GraphEase.slow:
      return u * u * u / 3.0;
    case GraphEase.fast:
      return u * u - u * u * u / 3.0;
    case GraphEase.smooth:
      if (u <= 0.5) return 2.0 * u * u * u / 3.0;
      final w = 1.0 - u;
      return u + 2.0 * w * w * w / 3.0 - 0.5;
    case GraphEase.sharp:
      if (u <= 0.5) return u * u - 2.0 * u * u * u / 3.0;
      return 2.0 * u * u * u / 3.0 - u * u + u - 1.0 / 6.0;
  }
}

/// The speed endpoints of a rate segment with the reverse gate applied: while
/// reverse is off, a negative speed evaluates as zero (§6.2). Every ease is
/// monotone, so clamping the endpoints clamps the whole profile.
(double, double) clampedSpeeds(double v0, double v1, bool allowReverse) {
  if (allowReverse) return (v0, v1);
  double floor(double v) => v < 0 ? 0 : v;
  return (floor(v0), floor(v1));
}

/// A cubic bezier over four scalar control points (Bernstein form) —
/// `retime.rs::bezier`.
double _bezier(List<double> p, double u) {
  final w = 1.0 - u;
  return w * w * w * p[0] +
      3.0 * w * w * u * p[1] +
      3.0 * w * u * u * p[2] +
      u * u * u * p[3];
}

/// The derivative of [_bezier] — `retime.rs::bezier_deriv`.
double _bezierDeriv(List<double> p, double u) {
  final w = 1.0 - u;
  return 3.0 * w * w * (p[1] - p[0]) +
      6.0 * w * u * (p[2] - p[1]) +
      3.0 * u * u * (p[3] - p[2]);
}

/// The §4.2 control points of a Map segment between its two boundaries, as
/// `([tx0..tx3], [sy0..sy3])` in (local seconds, source seconds) —
/// `retime.rs::map_control_points`.
(List<double>, List<double>) _mapControlPoints(
    BridgeRetimeSegment seg, BridgeRetimeBoundary lo, BridgeRetimeBoundary hi) {
  final t0 = lo.tSeconds, s0 = lo.sSeconds;
  final t1 = hi.tSeconds, s1 = hi.sSeconds;
  final d = t1 - t0;
  final m0 = seg.m0 ?? 0, m1 = seg.m1 ?? 0;
  final b0 = seg.b0 ?? (1 / 3), b1 = seg.b1 ?? (1 / 3);
  return (
    [t0, t0 + b0 * d, t1 - b1 * d, t1],
    [s0, s0 + m0 * b0 * d, s1 - m1 * b1 * d, s1],
  );
}

bool _isOneThird(double v) => (v - 1.0 / 3.0).abs() < 1e-9;

/// Find the bezier parameter u with x(u) = t — `retime.rs::map_param_at`
/// (linear when the handles are the polynomial 1/3, else a Newton-in-bracket
/// solve, `retime.rs::solve_u`).
double _mapParamAt(BridgeRetimeSegment seg, List<double> x, double t) {
  if (_isOneThird(seg.b0 ?? (1 / 3)) && _isOneThird(seg.b1 ?? (1 / 3))) {
    final span = x[3] - x[0];
    if (span <= 0) return 0;
    return ((t - x[0]) / span).clamp(0.0, 1.0);
  }
  return _solveU(x, t);
}

/// Solve x(u) = t by Newton inside a shrinking bisection bracket —
/// `retime.rs::solve_u` (the same solver as `anim::CubicSpan::solve_u`), run to
/// the ≤ 2⁻⁴⁸ relative tolerance of docs/04-RETIMING.md §4.3.
double _solveU(List<double> x, double t) {
  final x0 = x[0], x3 = x[3];
  if (x3 <= x0) return 0;
  final tol = (x3 - x0) * math.pow(2.0, -48);
  var lo = 0.0, hi = 1.0;
  var u = ((t - x0) / (x3 - x0)).clamp(0.0, 1.0);
  for (var i = 0; i < 48; i++) {
    final xu = _bezier(x, u);
    if ((xu - t).abs() <= tol) break;
    if (xu < t) {
      lo = u;
    } else {
      hi = u;
    }
    final dxu = _bezierDeriv(x, u);
    final newton = u - (xu - t) / dxu;
    u = (dxu > 1e-12 && newton > lo && newton < hi) ? newton : 0.5 * (lo + hi);
  }
  return u;
}

/// One sampled point of the speed lens: comp [frame] on the x axis and the
/// instantaneous speed in per cent on the y axis (100 = source rate).
typedef SpeedSample = ({double frame, double pct});

/// Sample the whole retime speed profile as a polyline of (comp frame, speed %)
/// points. Rate segments draw their native ease shape (two endpoint levels
/// joined by e(u)); Map segments draw their derivative y′(u)/x′(u). The x of a
/// point is the comp frame its local time maps to (linear within a segment, so
/// the join sits exactly on its boundary frame). Returns an empty list for a
/// structurally unusable store, never a throw.
List<SpeedSample> sampleSpeedCurve(BridgeRetime retime, {int perSegment = 24}) {
  final out = <SpeedSample>[];
  final bs = retime.boundaries;
  final segs = retime.segments;
  if (bs.length < 2 || segs.length != bs.length - 1) return out;
  final n = math.max(2, perSegment);
  for (var i = 0; i < segs.length; i++) {
    final lo = bs[i], hi = bs[i + 1];
    final seg = segs[i];
    final f0 = lo.tFrame.toDouble(), f1 = hi.tFrame.toDouble();
    final dFrame = f1 - f0;
    if (seg.kind == 'map') {
      final (x, y) = _mapControlPoints(seg, lo, hi);
      final tSpan = hi.tSeconds - lo.tSeconds;
      for (var k = (i == 0) ? 0 : 1; k <= n; k++) {
        final u = k / n;
        final t = _bezier(x, u);
        final frac = tSpan.abs() < 1e-12 ? u : (t - lo.tSeconds) / tSpan;
        final frame = f0 + frac * dFrame;
        final dx = _bezierDeriv(x, u);
        final speed = _bezierDeriv(y, u) / (dx.abs() < 1e-12 ? 1e-12 : dx);
        out.add((frame: frame, pct: speed * 100.0));
      }
    } else {
      final (v0, v1) =
          clampedSpeeds(seg.v0 ?? 1, seg.v1 ?? 1, retime.reverse);
      final ease = easeFromName(seg.ease);
      for (var k = (i == 0) ? 0 : 1; k <= n; k++) {
        final u = k / n;
        final frame = f0 + u * dFrame;
        final speed = v0 + (v1 - v0) * smallE(ease, u);
        out.add((frame: frame, pct: speed * 100.0));
      }
    }
  }
  return out;
}

/// The index of the segment whose comp-frame span `[boundaries[i].tFrame,
/// boundaries[i+1].tFrame)` contains [frame] (the last segment claims its own
/// end). Null for a structurally unusable store or a frame outside the domain.
/// This is the segment the preset row and →Rate act on when the playhead sits
/// on [frame].
int? segmentIndexAtFrame(BridgeRetime retime, int frame) {
  final bs = retime.boundaries;
  final segs = retime.segments;
  if (bs.length < 2 || segs.length != bs.length - 1) return null;
  if (frame < bs.first.tFrame || frame > bs.last.tFrame) return null;
  for (var i = 0; i < segs.length; i++) {
    final start = bs[i].tFrame;
    final end = bs[i + 1].tFrame;
    if (frame >= start && (frame < end || i == segs.length - 1)) return i;
  }
  return null;
}

/// The instantaneous speed in per cent at comp [frame] (the header readout).
/// Zero for a structurally unusable store or a frame outside the domain.
double speedPctAtFrame(BridgeRetime retime, int frame) {
  final i = segmentIndexAtFrame(retime, frame);
  if (i == null) return 0;
  final lo = retime.boundaries[i], hi = retime.boundaries[i + 1];
  final seg = retime.segments[i];
  final span = (hi.tFrame - lo.tFrame).toDouble();
  final u = span.abs() < 1e-9
      ? 0.0
      : ((frame - lo.tFrame) / span).clamp(0.0, 1.0);
  if (seg.kind == 'map') {
    final (x, y) = _mapControlPoints(seg, lo, hi);
    final tSpan = hi.tSeconds - lo.tSeconds;
    final t = lo.tSeconds + u * tSpan;
    final param = _mapParamAt(seg, x, t);
    final dx = _bezierDeriv(x, param);
    return _bezierDeriv(y, param) / (dx.abs() < 1e-12 ? 1e-12 : dx) * 100.0;
  }
  final (v0, v1) = clampedSpeeds(seg.v0 ?? 1, seg.v1 ?? 1, retime.reverse);
  return (v0 + (v1 - v0) * smallE(easeFromName(seg.ease), u)) * 100.0;
}

/// The interior boundary indices — the ones a drag may move. The first and last
/// boundaries are the clip's own domain ends (docs/04-RETIMING.md §3 pins the
/// start at local time 0), so only `1..n-2` are draggable; a single-segment
/// store therefore has none.
List<int> draggableBoundaryIndices(BridgeRetime retime) => [
      for (var i = 1; i < retime.boundaries.length - 1; i++) i,
    ];

/// The interior boundary whose drawn vertical is within [thresholdPx] of the
/// pointer at [pointerX], or null. [xOfFrame] maps a boundary's comp frame to
/// the same x the ruler uses. Ties go to the nearer boundary.
int? boundaryAtX(
  BridgeRetime retime,
  double pointerX,
  double Function(num frame) xOfFrame, {
  double thresholdPx = 6,
}) {
  int? best;
  var bestDist = thresholdPx;
  for (final i in draggableBoundaryIndices(retime)) {
    final d = (xOfFrame(retime.boundaries[i].tFrame) - pointerX).abs();
    if (d <= bestDist) {
      bestDist = d;
      best = i;
    }
  }
  return best;
}

/// Clamp a dragged boundary's target comp [frame] between its neighbours, one
/// frame clear of each (horizontal boundary drags are clamped between
/// neighbouring boundaries, docs/04-RETIMING.md §9). [index] must be interior.
int clampBoundaryFrame(BridgeRetime retime, int index, int frame) {
  final bs = retime.boundaries;
  final lo = bs[index - 1].tFrame + 1;
  final hi = bs[index + 1].tFrame - 1;
  if (hi < lo) return bs[index].tFrame; // no room; leave it put
  return frame.clamp(lo, hi);
}

/// A copy of [retime] with boundary [index]'s comp frame moved to [frame] — the
/// live-preview store drawn while a boundary drag is in flight (only the x of
/// the join moves; the segment speeds are untouched, matching the speed lens).
BridgeRetime withBoundaryFrame(BridgeRetime retime, int index, int frame) {
  final bs = [
    for (var i = 0; i < retime.boundaries.length; i++)
      if (i == index)
        BridgeRetimeBoundary(
          tFrame: frame,
          tSeconds: retime.boundaries[i].tSeconds,
          sSeconds: retime.boundaries[i].sSeconds,
          smooth: retime.boundaries[i].smooth,
        )
      else
        retime.boundaries[i],
  ];
  return BridgeRetime(
    reverse: retime.reverse,
    interpolation: retime.interpolation,
    boundaries: bs,
    segments: retime.segments,
  );
}

/// The source position (seconds) at local comp time [tSecs] — a port of
/// `retime.rs::Retime::evaluate`: a Rate segment maps `s_i + d·[v0·u +
/// (v1−v0)·E(u)]`, a Map segment `bezier(y, u(t))`. `tSecs` is clamped into the
/// store's local domain. Holds the first boundary's source position for a
/// structurally unusable store (never throws).
double sourceSecsAtLocal(BridgeRetime retime, double tSecs) {
  final bs = retime.boundaries;
  final segs = retime.segments;
  if (bs.length < 2 || segs.length != bs.length - 1) {
    return bs.isEmpty ? 0.0 : bs.first.sSeconds;
  }
  final t = tSecs.clamp(bs.first.tSeconds, bs.last.tSeconds);
  // Largest segment whose start boundary is <= t.
  var idx = 0;
  for (var i = 0; i < bs.length; i++) {
    if (bs[i].tSeconds <= t) idx = i;
  }
  final i = idx.clamp(0, segs.length - 1);
  final lo = bs[i], hi = bs[i + 1];
  final d = hi.tSeconds - lo.tSeconds;
  if (d <= 0) return lo.sSeconds;
  final seg = segs[i];
  if (seg.kind == 'map') {
    final (x, y) = _mapControlPoints(seg, lo, hi);
    return _bezier(y, _mapParamAt(seg, x, t));
  }
  final u = ((t - lo.tSeconds) / d).clamp(0.0, 1.0);
  final (v0, v1) = clampedSpeeds(seg.v0 ?? 1, seg.v1 ?? 1, retime.reverse);
  return lo.sSeconds + d * (v0 * u + (v1 - v0) * bigE(easeFromName(seg.ease), u));
}

/// The local comp time (seconds) at which the source is exhausted — a port of
/// `retime.rs::Retime::overrun_local_time`: the first boundary whose source
/// position reaches [sourceDurationSecs], then a bisection back to the exact
/// crossing. `0` when the clip starts already past the source end; null when the
/// source lasts to the out point (no overrun).
double? overrunLocalTime(BridgeRetime retime, double sourceDurationSecs) {
  final bs = retime.boundaries;
  final segs = retime.segments;
  if (bs.length < 2 || segs.length != bs.length - 1) return null;
  final dur = sourceDurationSecs;
  for (var i = 0; i < bs.length; i++) {
    if (bs[i].sSeconds < dur) continue;
    if (i == 0) return 0.0; // starts already past the source end
    var lo = bs[i - 1].tSeconds;
    var hi = bs[i].tSeconds;
    for (var k = 0; k < 40; k++) {
      final mid = 0.5 * (lo + hi);
      if (sourceSecsAtLocal(retime, mid) >= dur) {
        hi = mid;
      } else {
        lo = mid;
      }
    }
    return hi;
  }
  return null;
}

/// Where a retimed footage layer runs out of source, as a comp-time span in
/// seconds `(start, out)` — a port of `speed_rows.rs::overrun_span_secs`: from
/// the exhaustion point (clamped to the in point) to the out point, or null when
/// the source lasts to the out point (or runs out only past it). Indication
/// only — the hatch never moves a boundary (K-022).
(double, double)? overrunSpanSecs(
  BridgeRetime retime,
  double sourceDurationSecs,
  double startOffsetSecs,
  double inPointSecs,
  double outPointSecs,
) {
  final local = overrunLocalTime(retime, sourceDurationSecs);
  if (local == null) return null;
  final start = math.max(startOffsetSecs + local, inPointSecs);
  return start < outPointSecs ? (start, outPointSecs) : null;
}

/// The auto-fit y-range (speed %) for a sampled curve: always framing the 0%
/// and 100% references, padded 12% like the egui speed lens. Returns
/// `(lo, hi)`.
(double, double) speedRange(List<SpeedSample> samples) {
  var lo = 0.0, hi = 100.0;
  for (final s in samples) {
    lo = math.min(lo, s.pct);
    hi = math.max(hi, s.pct);
  }
  final pad = math.max((hi - lo).abs(), 1.0) * 0.12;
  return (lo - pad, hi + pad);
}
