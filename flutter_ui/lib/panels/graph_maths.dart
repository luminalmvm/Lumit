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

import 'package:lumit_flutter/src/rust/api/effect.dart';

/// A rational time as plain seconds — the evaluation domain (docs/14 §2:
/// authoritative times stay rational; evaluation converts once).
double rationalSeconds(BridgeRational r) => r.num / r.den.toDouble();

/// The AE easy-ease constant (F9): speed 0, influence one third.
const BridgeSideInterp easyEase = BridgeSideInterp.bezier(
  BridgeBezierSide(speed: 0, influence: 1 / 3),
);

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
}

/// A side's effective (speed, influence): a bezier side carries its own; a
/// linear (or hold-in) side lies on the chord with influence ⅓ (anim.rs
/// `side_params`).
(double, double) sideParams(BridgeSideInterp side, double chordSlope) =>
    switch (side) {
      BridgeSideInterp_Bezier(:final field0) => (
          field0.speed,
          field0.influence.clamp(1e-3, 1.0)
        ),
      _ => (chordSlope, 1.0 / 3.0),
    };

/// The span of a sorted key list holding `t` — its two keys and their times.
/// Callers have already handled `t` outside the keys, so a span always exists.
(BridgeKeyframe, BridgeKeyframe, double, double) _spanAt(
    List<BridgeKeyframe> keys, double t) {
  var idx = keys.length - 2;
  for (var i = 0; i + 1 < keys.length; i++) {
    if (t < rationalSeconds(keys[i + 1].time)) {
      idx = i;
      break;
    }
  }
  final a = keys[idx], b = keys[idx + 1];
  return (a, b, rationalSeconds(a.time), rationalSeconds(b.time));
}

/// The cubic across one span, its sides read through [sideParams].
CubicSpan _cubicOf(BridgeKeyframe a, BridgeKeyframe b, double t1, double t2) {
  final chord = (b.value - a.value) / (t2 - t1);
  final (s1, b1) = sideParams(a.interpOut, chord);
  final (s2, b2) = sideParams(b.interpIn, chord);
  return CubicSpan.fromAe(t1, a.value, t2, b.value,
      speedOut: s1, inflOut: b1, speedIn: s2, inflIn: b2);
}

/// Evaluate a sorted key list at `t` seconds — the engine's `evaluate`,
/// clamped past the ends, hold-out winning its span.
double evaluateKeys(List<BridgeKeyframe> keys, double t) {
  if (keys.isEmpty) return 0;
  if (t <= rationalSeconds(keys.first.time)) return keys.first.value;
  if (t >= rationalSeconds(keys.last.time)) return keys.last.value;
  final (a, b, t1, t2) = _spanAt(keys, t);
  final dt = t2 - t1;
  if (dt <= 0) return a.value;
  if (a.interpOut is BridgeSideInterp_Hold) return a.value;
  if (a.interpOut is BridgeSideInterp_Linear &&
      b.interpIn is BridgeSideInterp_Linear) {
    return a.value + (b.value - a.value) * ((t - t1) / dt);
  }
  return _cubicOf(a, b, t1, t2).valueAt(t);
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
  final (a, b, t1, t2) = _spanAt(keys, t);
  final dt = t2 - t1;
  if (dt <= 0) return 0;
  if (a.interpOut is BridgeSideInterp_Hold) return 0;
  if (a.interpOut is BridgeSideInterp_Linear &&
      b.interpIn is BridgeSideInterp_Linear) {
    return (b.value - a.value) / dt;
  }
  return _cubicOf(a, b, t1, t2).speedAt(t);
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
    BridgeSideInterp_Hold() => 0,
    _ => chord,
  };
}

/// A side's influence, defaulting to ⅓ where the side is not a bezier —
/// what a handle shows before its first drag.
double sideInfluence(BridgeSideInterp side) => switch (side) {
      BridgeSideInterp_Bezier(:final field0) =>
        field0.influence.clamp(1e-3, 1.0).toDouble(),
      _ => 1.0 / 3.0,
    };

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
        if (side is! BridgeSideInterp_Bezier) continue;
        final e = handleEndpoint(
          keyTime: t,
          keyValue: k.value,
          neighbourTime: rationalSeconds(nb.time),
          isOut: isOut,
          speed: side.field0.speed,
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
(double, double) fitSpeedRange(List<List<BridgeKeyframe>> channels) {
  var lo = 0.0, hi = 0.0;
  void grow(double v) {
    if (v < lo) lo = v;
    if (v > hi) hi = v;
  }

  for (final keys in channels) {
    for (var i = 0; i < keys.length; i++) {
      grow(sideSpeedAtKey(keys, i, isOut: true));
      grow(sideSpeedAtKey(keys, i, isOut: false));
    }
    if (keys.length >= 2) {
      final t0 = rationalSeconds(keys.first.time);
      final t1 = rationalSeconds(keys.last.time);
      for (var s = 0; s <= 64; s++) {
        grow(evaluateKeysSpeed(keys, t0 + (t1 - t0) * s / 64));
      }
    }
  }
  final pad = ((hi - lo).abs() < 1e-9 ? 1.0 : hi - lo) * 0.15;
  return (lo - pad, hi + pad);
}

// ---------------------------------------------------------------------------
// The keyframe clipboard text (docs/07 §5.3, K-196).
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

/// How a side's easing is written in the clipboard text: `linear`, `hold`, or
/// `bezier(speed,influence)`.
String easeToText(BridgeSideInterp side) => switch (side) {
      BridgeSideInterp_Hold() => 'hold',
      BridgeSideInterp_Bezier(:final field0) =>
        'bezier(${_number(field0.speed)},${_number(field0.influence)})',
      _ => 'linear',
    };

/// The easing a clipboard cell names, defaulting to linear for anything this
/// build does not recognise — including a table from another editor, which
/// carries values and no easing at all.
BridgeSideInterp easeFromText(String text) {
  final cell = text.trim().toLowerCase();
  if (cell == 'hold') return const BridgeSideInterp.hold();
  if (cell.startsWith('bezier(') && cell.endsWith(')')) {
    final parts = cell.substring(7, cell.length - 1).split(',');
    if (parts.length == 2) {
      final speed = double.tryParse(parts[0]);
      final influence = double.tryParse(parts[1]);
      if (speed != null && influence != null) {
        return BridgeSideInterp.bezier(BridgeBezierSide(
            speed: speed, influence: influence.clamp(1e-3, 1.0)));
      }
    }
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
// The Vegas speed envelope (K-247).
// ---------------------------------------------------------------------------

/// The vertical range a Retime channel's envelope opens at, in per cent
/// (K-247, K-250).
///
/// Headroom over normal playback, and enough below zero to show that dragging
/// a point down there runs the clip backwards. The room above 100% is the
/// point of the top figure: at exactly 100 the flat line every un-retimed clip
/// draws sat on the very top edge of the graph, with nowhere to go but down —
/// which reads as a ceiling rather than as the ordinary speed it is. It only
/// ever grows: a curve reaching past either end reframes the axis.
const (double, double) envelopeDefaultRange = (-25.0, 125.0);

/// The influence that makes a cubic side lie on its chord — the polynomial
/// subclass (K-078). The envelope authors every side at this influence, which
/// is what makes its straight lines exactly straight (see [envelopeToKeys]).
const double _chordInfluence = 1 / 3;

/// The envelope's points for [keys]: the playback speed at each key, in per
/// cent, where 100 is source rate and a negative value runs backwards.
///
/// A key's two sides carry a speed each, and an envelope point is a key whose
/// two agree ([envelopeToKeys] writes them that way). A channel shaped in the
/// Time lens instead can disagree at a key; the point then reads the side
/// facing *into* the clip — the outgoing speed, or the incoming one at the
/// last key — because that is the speed the span it governs is played at.
List<double> envelopeSpeeds(List<BridgeKeyframe> keys) => [
      for (var i = 0; i < keys.length; i++)
        sideSpeedAtKey(keys, i, isOut: i < keys.length - 1) * 100,
    ];

/// Keys carrying [speeds] (per cent, one per key), with the source positions
/// re-integrated from them and the **first key pinned**.
///
/// This is the Vegas edit: change a speed and the frames after it change,
/// while every keyframe *time* and the layer's own box stay exactly where they
/// are (K-022's covenant, K-070's start-pinning). The first key is what
/// "pinned" means — a clip's first frame is its own trim-in whatever its
/// speed, so re-speeding never moves where it starts.
///
/// **Why the trapezoid is exact rather than an approximation.** Between two
/// points the envelope draws a straight line, so the source advanced across a
/// span is the area under that line: the average of the two speeds times the
/// span. Setting a cubic's value change to exactly that, with its endpoint
/// slopes at the two speeds and influence ⅓, makes the cubic's own derivative
/// come out *exactly* that straight line — the u² term cancels. So the
/// envelope is not a simplified view of the curve underneath; it is the same
/// curve, read the other way round. (Substitute Δ/d = (m₀+m₁)/2 into a cubic
/// Hermite's derivative and the quadratic coefficient is zero.)
List<BridgeKeyframe> envelopeToKeys(
    List<BridgeKeyframe> keys, List<double> speeds) {
  if (keys.isEmpty || speeds.length != keys.length) return keys;
  // Every key's new source position first, so each side can be compared
  // against the chord of the span it governs.
  final values = <double>[keys.first.value];
  for (var i = 1; i < keys.length; i++) {
    final dt = rationalSeconds(keys[i].time) - rationalSeconds(keys[i - 1].time);
    // Non-positive only for keys sharing a time, which the editing ops
    // forbid; treated as no advance rather than as a reason to fail.
    values.add(dt > 0
        ? values[i - 1] + (speeds[i - 1] + speeds[i]) / 2 / 100 * dt
        : values[i - 1]);
  }

  /// The side governing the span between `a` and `b`, as seen from whichever
  /// end this is. **Linear when the speed is the span's own chord** — the two
  /// are then the same curve, and writing the bezier form anyway would change
  /// how the key *looks* (a diamond becomes a circle) for no change in what it
  /// does. Dragging one point of a flat envelope used to re-shape every key on
  /// the channel that way.
  BridgeSideInterp side(int at, int a, int b) {
    final dt = rationalSeconds(keys[b].time) - rationalSeconds(keys[a].time);
    final chord = dt > 0 ? (values[b] - values[a]) / dt : 0.0;
    final speed = speeds[at] / 100;
    if ((speed - chord).abs() < 1e-9) return const BridgeSideInterp.linear();
    return BridgeSideInterp.bezier(
      BridgeBezierSide(speed: speed, influence: _chordInfluence),
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
            : side(i, i - 1, i),
        interpOut: i == keys.length - 1
            ? const BridgeSideInterp.linear()
            : side(i, i, i + 1),
      ),
  ];
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
  return envelopeToKeys(moved, speeds);
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
