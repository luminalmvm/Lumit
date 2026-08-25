// The tone-curve editor (K-412, docs/08 §3.30) — the control a `Curve`
// parameter asks for.
//
// **In plain terms.** A curve parameter is a handful of points in a square. The
// bottom-left corner is black, the top-right is white; the line through the
// points says what an input brightness comes out as. Dragging a point up in the
// middle brightens the mid-tones, dragging it down darkens them, and the line
// bends smoothly rather than kinking because the points are joined by a spline.
//
// **The spline drawn here is for display only, and that is deliberate.** The
// engine fits the same clamped cubic once per resolve and bakes it into a
// 257-entry table, and *that* table is the picture (K-412). Re-implementing the
// fit in Dart is a divergence with its eyes open: what the user sees while
// dragging is a drawing of the curve, never the thing that grades the frame, so
// a last-bit disagreement between the two costs nothing and asking the engine
// for a table per pointer move would cost a bridge call per frame. The maths
// below is the same algorithm — end slopes clamped to the end secants, interior
// slopes from the C² tridiagonal system, samples clamped into the square — so
// the drawing and the grade agree to well within a pixel of the plot.
//
// Bridge-free on purpose: it takes and returns plain `[x, y]` lists, so the
// panel row owns every crossing and this owns the gesture.

import 'dart:math' as math;

import 'package:flutter/gestures.dart' show DragStartBehavior;
import 'package:flutter/widgets.dart';

import 'controls.dart';

/// The most control points a curve may carry (docs/08 §1.2). The engine drops
/// the tail past this on read; the editor simply stops adding.
const int curveMaxPoints = 16;

/// The identity diagonal — a fresh curve, and what Reset restores.
const List<List<double>> curveIdentity = [
  [0.0, 0.0],
  [1.0, 1.0]
];

/// How far outside the plot a point must be dragged before it is dropped, in
/// logical pixels. Generous, because losing a point to a twitchy hand is worse
/// than having to mean it.
const double _dropDistance = 34;

/// How near the pointer must come to grab an existing point rather than add
/// one, in logical pixels.
const double _grabRadius = 9;

/// The smallest gap in x between two neighbouring points. Two points at one x
/// are a vertical wall the spline cannot describe — and the engine keeps only
/// the first of them — so the drag stops just short of it.
const double _minGap = 0.004;

/// Sample the clamped cubic through [points] at [x] — **display only**, see the
/// file header.
///
/// Points are assumed sorted by x with at least two entries, which is what the
/// editor always holds. Out-of-range x extrapolates along the end segment,
/// exactly as the engine's lookup does.
double curveSample(List<List<double>> points, double x) {
  final n = points.length;
  if (n < 2) return x;
  final m = _slopes(points);
  // Which segment x falls in, walking forward: sixteen points at most.
  var i = 0;
  while (i < n - 2 && x > points[i + 1][0]) {
    i++;
  }
  final x0 = points[i][0], x1 = points[i + 1][0];
  final y0 = points[i][1], y1 = points[i + 1][1];
  final h = x1 - x0;
  if (h <= 0) return y0.clamp(0.0, 1.0);
  final t = (x - x0) / h;
  final t2 = t * t, t3 = t2 * t;
  // Hermite form.
  final y = (2 * t3 - 3 * t2 + 1) * y0 +
      (t3 - 2 * t2 + t) * h * m[i] +
      (-2 * t3 + 3 * t2) * y1 +
      (t3 - t2) * h * m[i + 1];
  return y.clamp(0.0, 1.0);
}

/// The end-clamped C² slopes at each point — a tridiagonal system solved by
/// Thomas, the engine's `cpu::curve_table` in Dart.
List<double> _slopes(List<List<double>> p) {
  final n = p.length;
  final h = List<double>.filled(n - 1, 0);
  final d = List<double>.filled(n - 1, 0);
  for (var i = 0; i < n - 1; i++) {
    h[i] = p[i + 1][0] - p[i][0];
    d[i] = h[i] == 0 ? 0 : (p[i + 1][1] - p[i][1]) / h[i];
  }
  final m = List<double>.filled(n, 0);
  // The end condition that makes a two-point curve its own straight line.
  m[0] = d[0];
  m[n - 1] = d[n - 2];
  if (n == 2) return m;

  // Interior rows: h[i]·m[i−1] + 2(h[i−1]+h[i])·m[i] + h[i−1]·m[i+1] = rhs,
  // with the two known end slopes moved to the right-hand side.
  final interior = n - 2;
  final a = List<double>.filled(interior, 0);
  final b = List<double>.filled(interior, 0);
  final c = List<double>.filled(interior, 0);
  final r = List<double>.filled(interior, 0);
  for (var k = 0; k < interior; k++) {
    final i = k + 1;
    a[k] = h[i];
    b[k] = 2 * (h[i - 1] + h[i]);
    c[k] = h[i - 1];
    r[k] = 3 * (h[i] * d[i - 1] + h[i - 1] * d[i]);
  }
  r[0] -= a[0] * m[0];
  a[0] = 0;
  r[interior - 1] -= c[interior - 1] * m[n - 1];
  c[interior - 1] = 0;

  for (var k = 1; k < interior; k++) {
    if (b[k - 1] == 0) continue;
    final f = a[k] / b[k - 1];
    b[k] -= f * c[k - 1];
    r[k] -= f * r[k - 1];
  }
  for (var k = interior - 1; k >= 0; k--) {
    final rest = k == interior - 1 ? 0.0 : c[k] * m[k + 2];
    m[k + 1] = b[k] == 0 ? 0 : (r[k] - rest) / b[k];
  }
  return m;
}

/// One channel's curve: the unit square, the spline, and the points on it.
///
/// A drag reports live through [onLive] and commits once on release through
/// [onCommit]; adding, removing and resetting commit at once, because there is
/// no gesture still in progress to wait for.
class CurveEditor extends StatefulWidget {
  final List<List<double>> points;

  /// A drag tick — preview it, do not commit it.
  final ValueChanged<List<List<double>>> onLive;

  /// The end of a gesture: commit this as one edit.
  final ValueChanged<List<List<double>>> onCommit;

  /// The plot's side, in logical pixels.
  final double size;

  /// What the curve itself is drawn in. Null is the theme's primary text
  /// colour, which is what a Master curve wants; a channel curve passes its
  /// own colour so a Red tab draws red (owner, desk test).
  final Color? line;

  const CurveEditor({
    super.key,
    required this.points,
    required this.onLive,
    required this.onCommit,
    this.size = 150,
    this.line,
  });

  @override
  State<CurveEditor> createState() => _CurveEditorState();
}

class _CurveEditorState extends State<CurveEditor> {
  /// The curve while a drag is in flight; null when the widget's own points are
  /// the truth.
  List<List<double>>? _dragging;

  /// Which point the drag has hold of, and whether it has been dropped out of
  /// the square (so the release commits the shorter list).
  int _held = -1;

  List<List<double>> get _points {
    final live = _dragging ?? widget.points;
    return live.length >= 2 ? live : curveIdentity;
  }

  /// Plot coordinates → curve coordinates. y is flipped: up is more light.
  List<double> _toCurve(Offset local) => [
        (local.dx / widget.size).clamp(0.0, 1.0),
        (1 - local.dy / widget.size).clamp(0.0, 1.0),
      ];

  Offset _toPlot(List<double> p) =>
      Offset(p[0] * widget.size, (1 - p[1]) * widget.size);

  /// How far outside the plot [local] is, in logical pixels — zero inside.
  double _outsideBy(Offset local) {
    final dx = math.max(math.max(-local.dx, local.dx - widget.size), 0.0);
    final dy = math.max(math.max(-local.dy, local.dy - widget.size), 0.0);
    return math.max(dx, dy);
  }

  int _nearest(Offset local) {
    var best = -1;
    var bestD = _grabRadius;
    final points = _points;
    for (var i = 0; i < points.length; i++) {
      final d = (_toPlot(points[i]) - local).distance;
      if (d <= bestD) {
        bestD = d;
        best = i;
      }
    }
    return best;
  }

  void _onTapUp(TapUpDetails d) {
    final local = d.localPosition;
    if (_nearest(local) >= 0) return; // a tap on a point is not a new point
    final points = _points;
    if (points.length >= curveMaxPoints) return;
    final at = _toCurve(local);
    final next = [
      for (final p in points) [p[0], p[1]]
    ];
    // In x order, and never on top of a neighbour.
    var i = 0;
    while (i < next.length && next[i][0] < at[0]) {
      i++;
    }
    if (i > 0 && (at[0] - next[i - 1][0]).abs() < _minGap) return;
    if (i < next.length && (next[i][0] - at[0]).abs() < _minGap) return;
    next.insert(i, at);
    widget.onCommit(next);
  }

  void _onPanStart(DragStartDetails d) {
    final i = _nearest(d.localPosition);
    if (i < 0) return;
    setState(() {
      _held = i;
      _dragging = [
        for (final p in _points) [p[0], p[1]]
      ];
    });
  }

  void _onPanUpdate(DragUpdateDetails d) {
    if (_held < 0) return;
    final points = _dragging;
    if (points == null) return;
    final at = _toCurve(d.localPosition);
    // An endpoint stays an endpoint but is not pinned to the corner: the black
    // and white points slide along their edge, which is the move that crushes
    // or lifts an end (After Effects allows it, and the engine's evaluation
    // extrapolates past them rather than clipping).
    final lo = _held == 0 ? 0.0 : points[_held - 1][0] + _minGap;
    final hi =
        _held == points.length - 1 ? 1.0 : points[_held + 1][0] - _minGap;
    setState(() {
      points[_held] = [at[0].clamp(math.min(lo, hi), math.max(lo, hi)), at[1]];
    });
    widget.onLive(_dropped(d.localPosition) ?? points);
  }

  /// The list this gesture would leave behind if it ended here: the point gone
  /// when it has been dragged well clear of the square, else null.
  List<List<double>>? _dropped(Offset local) {
    final points = _dragging;
    if (points == null || _held < 0) return null;
    // The two ends are never removed — a curve needs somewhere to start and
    // finish, and After Effects does not remove them either.
    if (_held == 0 || _held == points.length - 1) return null;
    if (_outsideBy(local) < _dropDistance) return null;
    return [
      for (var i = 0; i < points.length; i++)
        if (i != _held) [points[i][0], points[i][1]]
    ];
  }

  void _onPanEnd(Offset? last) {
    final points = _dragging;
    if (points != null && _held >= 0) {
      widget.onCommit(last == null ? points : (_dropped(last) ?? points));
    }
    setState(() {
      _dragging = null;
      _held = -1;
    });
  }

  Offset? _lastLocal;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    void update(DragUpdateDetails d) {
      _lastLocal = d.localPosition;
      _onPanUpdate(d);
    }

    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      // The drag starts where the *pointer went down*, not where the slop was
      // finally exceeded: the grab has to test against the point the user
      // actually aimed at, and `DragStartBehavior.start` reports a position
      // already twenty pixels away from it.
      dragStartBehavior: DragStartBehavior.down,
      onTapUp: _onTapUp,
      // **Two axis recognisers rather than one pan**, because the panel this
      // sits in is a list. A pan needs twice the slop a single-axis drag does,
      // so an enclosing vertical scroll always wins the arena first and every
      // upward drag on a point scrolled the panel instead of lifting the
      // curve. An axis recogniser meets the scroll on equal terms and the
      // inner one wins, which is the same arrangement a slider in a list uses.
      // Whichever axis crosses first carries the whole gesture, and its
      // details still report both coordinates, so a diagonal drag is not
      // flattened onto one axis.
      onVerticalDragStart: _onPanStart,
      onVerticalDragUpdate: update,
      onVerticalDragEnd: (_) => _onPanEnd(_lastLocal),
      onVerticalDragCancel: () => _onPanEnd(null),
      onHorizontalDragStart: _onPanStart,
      onHorizontalDragUpdate: update,
      onHorizontalDragEnd: (_) => _onPanEnd(_lastLocal),
      onHorizontalDragCancel: () => _onPanEnd(null),
      child: SizedBox(
        width: widget.size,
        height: widget.size,
        child: CustomPaint(
          painter: _CurvePainter(
            points: _points,
            plot: t.surface0,
            grid: t.hairline,
            diagonal: t.hairlineStrong,
            line: widget.line ?? t.textPrimary,
            knob: t.accent,
          ),
        ),
      ),
    );
  }
}

class _CurvePainter extends CustomPainter {
  final List<List<double>> points;
  final Color plot, grid, diagonal, line, knob;

  const _CurvePainter({
    required this.points,
    required this.plot,
    required this.grid,
    required this.diagonal,
    required this.line,
    required this.knob,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final rect = Offset.zero & size;
    canvas.drawRect(rect, Paint()..color = plot);

    final hair = Paint()
      ..color = grid
      ..strokeWidth = 1;
    for (var i = 1; i < 4; i++) {
      final f = i / 4;
      canvas.drawLine(
          Offset(size.width * f, 0), Offset(size.width * f, size.height), hair);
      canvas.drawLine(Offset(0, size.height * f),
          Offset(size.width, size.height * f), hair);
    }
    // The identity, so a bent curve reads as bent away from something.
    canvas.drawLine(
      Offset(0, size.height),
      Offset(size.width, 0),
      Paint()
        ..color = diagonal
        ..strokeWidth = 1,
    );

    // The spline, sampled a pixel at a time across the plot.
    final path = Path();
    final steps = math.max(size.width.round(), 2);
    for (var i = 0; i <= steps; i++) {
      final x = i / steps;
      final y = curveSample(points, x);
      final at = Offset(x * size.width, (1 - y) * size.height);
      i == 0 ? path.moveTo(at.dx, at.dy) : path.lineTo(at.dx, at.dy);
    }
    canvas.drawPath(
      path,
      Paint()
        ..color = line
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.5,
    );

    final fill = Paint()..color = knob;
    for (final p in points) {
      final at = Offset(p[0] * size.width, (1 - p[1]) * size.height);
      canvas.drawRect(Rect.fromCenter(center: at, width: 6, height: 6), fill);
    }
  }

  @override
  bool shouldRepaint(_CurvePainter old) =>
      old.line != line || old.knob != knob || !_same(old.points, points);

  static bool _same(List<List<double>> a, List<List<double>> b) {
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (a[i][0] != b[i][0] || a[i][1] != b[i][1]) return false;
    }
    return true;
  }
}

/// Several curves behind channel tabs — Curves' five, drawn as one editor
/// rather than five stacked widgets (K-412, docs/08 §3.30).
///
/// [labels] names the tabs; [curves] is one point list each, same order.
class CurveChannelEditor extends StatefulWidget {
  final List<String> labels;
  final List<List<List<double>>> curves;

  /// A drag tick on the channel at this index.
  final void Function(int channel, List<List<double>> points) onLive;

  /// A committed edit on the channel at this index.
  final void Function(int channel, List<List<double>> points) onCommit;

  /// A stable prefix for the tab and plot keys, so a test can point at one.
  final String keyPrefix;

  /// The word on the per-channel reset action.
  final String resetLabel;

  /// Its tooltip.
  final String resetTip;

  /// What each channel's curve draws in, parallel to [labels]. A null entry —
  /// or no list at all — leaves that channel on the theme's own colour, which
  /// is what Master and Alpha want; Red, Green and Blue pass theirs, because a
  /// channel curve that is not its own colour is a graph you have to read the
  /// tab strip to understand (owner, desk test).
  final List<Color?>? channelColours;

  const CurveChannelEditor({
    super.key,
    required this.labels,
    required this.curves,
    required this.onLive,
    required this.onCommit,
    required this.keyPrefix,
    required this.resetLabel,
    required this.resetTip,
    this.channelColours,
  });

  @override
  State<CurveChannelEditor> createState() => _CurveChannelEditorState();
}

class _CurveChannelEditorState extends State<CurveChannelEditor> {
  int _channel = 0;

  /// The channel's own colour, or null for the ones drawn in the theme's.
  Color? _colourOf(int channel) {
    final colours = widget.channelColours;
    if (colours == null) return null;
    if (channel < 0 || channel >= colours.length) return null;
    return colours[channel];
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final channel = _channel.clamp(0, widget.curves.length - 1);
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Row(
            children: [
              for (var i = 0; i < widget.labels.length; i++)
                Padding(
                  padding: const EdgeInsets.only(right: 2),
                  child: HouseButton(
                    key: ValueKey<String>('${widget.keyPrefix}-tab-$i'),
                    frameless: i != channel,
                    small: true,
                    padding:
                        const EdgeInsets.symmetric(horizontal: 5, vertical: 1),
                    onPressed: () => setState(() => _channel = i),
                    child: Text(
                      widget.labels[i],
                      style: t.small.copyWith(
                        color: i == channel
                            ? (_colourOf(i) ?? t.textPrimary)
                            : t.textMuted,
                      ),
                    ),
                  ),
                ),
              const Spacer(),
              LumitTooltip(
                message: widget.resetTip,
                child: HouseButton(
                  key: ValueKey<String>('${widget.keyPrefix}-reset'),
                  frameless: true,
                  small: true,
                  padding:
                      const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
                  onPressed: () => widget.onCommit(channel, curveIdentity),
                  child: Text(widget.resetLabel,
                      style: t.small.copyWith(color: t.textMuted)),
                ),
              ),
            ],
          ),
          const SizedBox(height: 4),
          CurveEditor(
            key: ValueKey<String>('${widget.keyPrefix}-plot-$channel'),
            points: widget.curves[channel],
            line: _colourOf(channel),
            onLive: (p) => widget.onLive(channel, p),
            onCommit: (p) => widget.onCommit(channel, p),
          ),
        ],
      ),
    );
  }
}
