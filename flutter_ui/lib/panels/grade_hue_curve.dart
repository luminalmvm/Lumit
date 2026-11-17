// The editor for a Correction's six hue curves: Hue v hue, Hue v sat,
// Hue v lum, Lum v sat, Sat v sat and Sat v lum.
//
// In plain terms: the line across the middle leaves the picture alone, and
// raising or lowering it at a point pushes that part of the picture one way.
// The three whose input is hue go round a circle, so the curve leaving the
// right edge comes back in at the left: the line is drawn and read wrapped,
// the way the engine bakes it. Under a hue curve sit the six vector colours;
// a click on one drops three points on that colour, ready to drag.

import 'dart:math' as math;

import 'package:flutter/gestures.dart' show DragStartBehavior;
import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../widgets/colour_picker.dart' show hsvToRgb;
import '../widgets/controls.dart';
import '../widgets/curve_editor.dart' show curveMaxPoints, curveSample;
import 'timeline_extras_frb.dart' show DoubleTap;

/// The hue the engine reads off a value in its grading space, 0 to 1 from
/// red through yellow, green, cyan, blue and magenta: the vectorscope's
/// angle, red at nought.
double gradeHueOf(double r, double g, double b) {
  final y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
  final cb = (b - y) / 1.8556;
  final cr = (r - y) / 1.5748;
  if (cb * cb + cr * cr < 1e-12) return 0;
  final turn = (math.atan2(cr, cb) - 1.7960518) / (2 * math.pi);
  final hue = turn - turn.floorToDouble();
  // Red lands a hair under a whole turn; it is the start of the circle, and
  // the strips under the curves need their stops in order from it.
  return hue > 1 - 1e-6 ? 0 : hue;
}

/// The six vector colours, as 0 to 1 channels, in hue order.
const List<List<double>> _vectors = [
  [1, 0, 0],
  [1, 1, 0],
  [0, 1, 0],
  [0, 1, 1],
  [0, 0, 1],
  [1, 0, 1],
];

/// Where each vector colour sits along a hue curve.
final List<double> gradeVectorHues = [
  for (final v in _vectors) gradeHueOf(v[0], v[1], v[2]),
];

/// The value a curve's points give at [x], read round the circle when
/// [periodic]: the engine's own wrap, a cubic through each span with the
/// slope of its two neighbours.
double hueCurveSample(List<List<double>> points, double x, bool periodic) {
  if (!periodic) return curveSample(points, x);
  final xs = _wrapped(points);
  final n = xs.length;
  if (n == 0) return 0.5;
  if (n == 1) return xs.first[1].clamp(0.0, 1.0);
  List<double> at(int k) {
    final wraps = (k / n).floor();
    final p = xs[k - wraps * n];
    return [p[0] + wraps, p[1]];
  }

  double slope(int k) {
    final a = at(k - 1), b = at(k + 1);
    return b[0] > a[0] ? (b[1] - a[1]) / (b[0] - a[0]) : 0;
  }

  var k = -1;
  while (at(k + 1)[0] <= x) {
    k++;
  }
  final p0 = at(k), p1 = at(k + 1);
  final span = p1[0] - p0[0];
  final t = span > 0 ? (x - p0[0]) / span : 0.0;
  final t2 = t * t, t3 = t2 * t;
  final y = p0[1] * (2 * t3 - 3 * t2 + 1) +
      slope(k) * span * (t3 - 2 * t2 + t) +
      p1[1] * (-2 * t3 + 3 * t2) +
      slope(k + 1) * span * (t3 - t2);
  return y.clamp(0.0, 1.0);
}

/// The points a periodic curve really has: those before 1, with a point at
/// 1 standing in for one at 0 when there is none, as the engine reads them.
List<List<double>> _wrapped(List<List<double>> points) {
  final out = <List<double>>[
    for (final p in points)
      if (p[0] < 1) [p[0], p[1]],
  ];
  if (!out.any((p) => p[0] == 0)) {
    for (final p in points) {
      if (p[0] >= 1) {
        out.insert(0, [0, p[1]]);
        break;
      }
    }
  }
  out.sort((a, b) => a[0].compareTo(b[0]));
  return out;
}

/// What the curve's input is, for the strip drawn under it.
enum HueCurveInput { hue, luma, saturation }

class GradeHueCurve extends StatefulWidget {
  final List<List<double>> points;
  final HueCurveInput input;
  final double width, height;
  final bool enabled;

  /// Whether the curve has keys, so its line wears the animated colour.
  final bool keyed;
  final ValueChanged<List<List<double>>> onLive;
  final ValueChanged<List<List<double>>> onCommit;

  /// What Reset goes back to.
  final List<List<double>> rest;

  /// Rows the panel adds to the right-click menu.
  final List<Widget> Function(VoidCallback close)? moreRows;

  const GradeHueCurve({
    super.key,
    required this.points,
    required this.input,
    required this.width,
    required this.height,
    required this.enabled,
    required this.keyed,
    required this.onLive,
    required this.onCommit,
    required this.rest,
    this.moreRows,
  });

  bool get periodic => input == HueCurveInput.hue;

  @override
  State<GradeHueCurve> createState() => GradeHueCurveState();
}

class GradeHueCurveState extends State<GradeHueCurve> {
  List<List<double>>? _dragging;
  int _held = -1;
  Offset? _last;

  static const double _grab = 7;
  static const double _gap = 0.01;
  static const double _drop = 32;
  static const double _strip = 8;

  double get _plotHeight => widget.height - _strip - 2;

  /// The points the editor shows and moves: those before 1 on a hue curve.
  List<List<double>> get _points {
    final live = _dragging;
    if (live != null) return live;
    return widget.periodic
        ? _wrapped(widget.points)
        : [
            for (final p in widget.points) [p[0], p[1]]
          ];
  }

  /// The list a commit sends: never fewer than two points, which the engine
  /// would read as a diagonal. A hue curve's second is its wrap at 1.
  List<List<double>> _out(List<List<double>> points) {
    if (!widget.periodic || points.length >= 2) return points;
    final y = points.isEmpty ? 0.5 : points.first[1];
    return [
      if (points.isEmpty) [0.0, y] else points.first,
      [1.0, y],
    ];
  }

  Offset _toPlot(List<double> p) =>
      Offset(p[0] * widget.width, (1 - p[1]) * _plotHeight);

  List<double> _toCurve(Offset local) => [
        (local.dx / widget.width).clamp(0.0, widget.periodic ? 0.999 : 1.0),
        (1 - local.dy / _plotHeight).clamp(0.0, 1.0),
      ];

  int _nearest(Offset local) {
    var best = -1;
    var bestD = _grab;
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

  double _yAt(double x) => hueCurveSample(_points, x, widget.periodic);

  /// Points at [xs] on the line as it stands, replacing any too close to
  /// one, so a drop moves nothing until a point is dragged.
  List<List<double>> withPointsAt(List<double> xs) {
    final next = [
      for (final p in _points)
        if (!xs.any((x) => _circleGap(p[0], x) < _gap * 2)) [p[0], p[1]]
    ];
    for (final x in xs) {
      final wrapped = widget.periodic ? x - x.floorToDouble() : x.clamp(0.0, 1.0);
      next.add([wrapped, _yAt(wrapped)]);
    }
    next.sort((a, b) => a[0].compareTo(b[0]));
    return next.length > curveMaxPoints ? _points : next;
  }

  double _circleGap(double a, double b) {
    final d = (a - b).abs();
    return widget.periodic ? math.min(d, 1 - d) : d;
  }

  /// Three points round [centre], [spread] either side, as one commit.
  void dropThree(double centre, double spread) => widget
      .onCommit(_out(withPointsAt([centre - spread, centre, centre + spread])));

  /// Counted by hand, so a single click adds its point at once.
  final DoubleTap _taps = DoubleTap();

  void _tap(TapUpDetails d) {
    final near = _nearest(d.localPosition);
    if (near >= 0) {
      if (_taps.tap(at: d.localPosition, slop: 6)) _flatten(near);
      return;
    }
    if (_points.length >= curveMaxPoints) return;
    final x = _toCurve(d.localPosition)[0];
    widget.onCommit(_out(withPointsAt([x])));
  }

  void _start(DragStartDetails d) {
    final i = _nearest(d.localPosition);
    if (i < 0) return;
    setState(() {
      _held = i;
      _dragging = [
        for (final p in _points) [p[0], p[1]]
      ];
    });
  }

  void _update(DragUpdateDetails d) {
    final points = _dragging;
    if (points == null || _held < 0) return;
    _last = d.localPosition;
    final at = _toCurve(d.localPosition);
    // A point keeps its place among the others; a tone curve's ends stay at
    // the edges, a hue curve's go where they are taken.
    final ends = !widget.periodic &&
        (_held == 0 || _held == points.length - 1);
    final lo = _held == 0 ? 0.0 : points[_held - 1][0] + _gap;
    final hi = _held == points.length - 1
        ? (widget.periodic ? 0.999 : 1.0)
        : points[_held + 1][0] - _gap;
    setState(() {
      points[_held] = [
        ends ? points[_held][0] : at[0].clamp(math.min(lo, hi), math.max(lo, hi)),
        at[1],
      ];
    });
    widget.onLive(_out(_dropped(d.localPosition) ?? points));
  }

  /// The list without the held point once it is dragged well clear of the
  /// plot, else null. A tone curve keeps its ends.
  List<List<double>>? _dropped(Offset local) {
    final points = _dragging;
    if (points == null || _held < 0) return null;
    if (!widget.periodic && (_held == 0 || _held == points.length - 1)) {
      return null;
    }
    final out = math.max(-local.dy, local.dy - _plotHeight);
    if (out < _drop) return null;
    return [
      for (var i = 0; i < points.length; i++)
        if (i != _held) points[i]
    ];
  }

  void _end(bool cancelled) {
    final points = _dragging;
    final last = _last;
    if (points != null && _held >= 0 && !cancelled) {
      widget.onCommit(
          _out(last == null ? points : (_dropped(last) ?? points)));
    }
    setState(() {
      _dragging = null;
      _held = -1;
      _last = null;
    });
  }

  void _menu(Offset local, Offset global) {
    final near = _nearest(local);
    final points = _points;
    final canDelete = near >= 0 &&
        (widget.periodic || (near > 0 && near < points.length - 1));
    final bent = !_same(_out(points), widget.rest);
    showLumitPopup<void>(
      context: context,
      position: global,
      builder: (close) {
        void done() => close(null);
        final more = widget.moreRows?.call(done) ?? const <Widget>[];
        return FloatSurface(
          child: IntrinsicWidth(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                MenuRow(
                  key: const ValueKey('hue-curve-delete'),
                  enabled: canDelete,
                  onPressed: () {
                    done();
                    widget.onCommit(_out([
                      for (var i = 0; i < points.length; i++)
                        if (i != near) points[i]
                    ]));
                  },
                  child: MenuLabel(l10n.curveDeletePoint, enabled: canDelete),
                ),
                MenuRow(
                  key: const ValueKey('hue-curve-reset'),
                  enabled: bent,
                  onPressed: () {
                    done();
                    widget.onCommit(widget.rest);
                  },
                  child: MenuLabel(l10n.tipResetCurve, enabled: bent),
                ),
                if (more.isNotEmpty) ...[const MenuDivider(), ...more],
              ],
            ),
          ),
        );
      },
    );
  }

  static bool _same(List<List<double>> a, List<List<double>> b) =>
      a.length == b.length &&
      [
        for (var i = 0; i < a.length; i++)
          (a[i][0] - b[i][0]).abs() < 1e-6 && (a[i][1] - b[i][1]).abs() < 1e-6
      ].every((x) => x);

  /// A double-click on a point puts it back on the middle line.
  void _flatten(int i) {
    final points = _points;
    if ((points[i][1] - 0.5).abs() < 1e-6) return;
    widget.onCommit(_out([
      for (var k = 0; k < points.length; k++)
        k == i ? [points[k][0], 0.5] : points[k]
    ]));
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final plot = CustomPaint(
      size: Size(widget.width, widget.height),
      painter: _HueCurvePainter(
        points: _points,
        input: widget.input,
        stripHeight: _strip,
        ground: t.surface1,
        grid: t.hairline,
        middle: t.hairlineStrong,
        line: !widget.enabled
            ? t.textDisabled
            : widget.keyed
                ? t.animated
                : t.textPrimary,
        knob: widget.enabled ? t.accent : t.textDisabled,
        enabled: widget.enabled,
      ),
    );
    if (!widget.enabled) return plot;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      dragStartBehavior: DragStartBehavior.down,
      onTapUp: _tap,
      onSecondaryTapUp: (d) => _menu(d.localPosition, d.globalPosition),
      // Two axis recognisers, not a pan, so the panel's scroll does not win
      // every upward drag (the curve editor's reasoning).
      onVerticalDragStart: _start,
      onVerticalDragUpdate: _update,
      onVerticalDragEnd: (_) => _end(false),
      onVerticalDragCancel: () => _end(true),
      onHorizontalDragStart: _start,
      onHorizontalDragUpdate: _update,
      onHorizontalDragEnd: (_) => _end(false),
      onHorizontalDragCancel: () => _end(true),
      child: MouseRegion(cursor: SystemMouseCursors.precise, child: plot),
    );
  }
}

class _HueCurvePainter extends CustomPainter {
  final List<List<double>> points;
  final HueCurveInput input;
  final double stripHeight;
  final Color ground, grid, middle, line, knob;
  final bool enabled;

  const _HueCurvePainter({
    required this.points,
    required this.input,
    required this.stripHeight,
    required this.ground,
    required this.grid,
    required this.middle,
    required this.line,
    required this.knob,
    required this.enabled,
  });

  bool get _periodic => input == HueCurveInput.hue;

  /// The strip's colours along x: the hue circle, a grey ramp for luma, or
  /// grey to a full colour for saturation. Grey where the editor is off.
  List<Color> get _stripColours => switch (input) {
        HueCurveInput.hue => [
            for (final v in [..._vectors, _vectors.first])
              _shade(hsvToRgb(_hueDegrees(v), 0.6, 0.85)),
          ],
        HueCurveInput.luma => [hsvToRgb(0, 0, 0.05), hsvToRgb(0, 0, 0.95)],
        HueCurveInput.saturation => [
            hsvToRgb(30, 0, 0.6),
            _shade(hsvToRgb(30, 0.75, 0.85))
          ],
      };

  List<double> get _stripStops => switch (input) {
        HueCurveInput.hue => [...gradeVectorHues, 1.0],
        _ => const [0.0, 1.0],
      };

  Color _shade(Color c) => enabled ? c : hsvToRgb(0, 0, 0.42);

  static double _hueDegrees(List<double> v) =>
      [0.0, 60.0, 120.0, 180.0, 240.0, 300.0][_vectors.indexOf(v) % 6];

  @override
  void paint(Canvas canvas, Size size) {
    final h = size.height - stripHeight - 2;
    final rect = Rect.fromLTWH(0, 0, size.width, h);
    canvas.drawRect(rect, Paint()..color = ground);
    final hair = Paint()
      ..color = grid
      ..strokeWidth = 1;
    for (var i = 1; i < 6; i++) {
      final x = size.width * i / 6;
      canvas.drawLine(Offset(x, 0), Offset(x, h), hair);
    }
    for (final f in const [0.25, 0.75]) {
      canvas.drawLine(Offset(0, h * f), Offset(size.width, h * f), hair);
    }
    canvas.drawLine(
      Offset(0, h / 2),
      Offset(size.width, h / 2),
      Paint()
        ..color = middle
        ..strokeWidth = 1,
    );
    canvas.drawRect(
      Rect.fromLTWH(0, h + 2, size.width, stripHeight),
      Paint()
        ..shader = LinearGradient(colors: _stripColours, stops: _stripStops)
            .createShader(Rect.fromLTWH(0, h + 2, size.width, stripHeight)),
    );
    final path = Path();
    final steps = math.max(size.width.round(), 2);
    for (var i = 0; i <= steps; i++) {
      final x = i / steps;
      final y = hueCurveSample(points, _periodic ? x.clamp(0.0, 0.9999) : x,
          _periodic);
      final at = Offset(x * size.width, (1 - y) * h);
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
      final at = Offset(p[0] * size.width, (1 - p[1]) * h);
      canvas.drawRect(Rect.fromCenter(center: at, width: 6, height: 6), fill);
    }
  }

  @override
  bool shouldRepaint(_HueCurvePainter old) =>
      old.line != line ||
      old.knob != knob ||
      old.input != input ||
      old.enabled != enabled ||
      !GradeHueCurveState._same(old.points, points);
}

/// The six vector colours as swatches: a click drops three points on that
/// colour's place along the curve.
class GradeVectorSwatches extends StatelessWidget {
  final bool enabled;
  final ValueChanged<double> onPick;

  const GradeVectorSwatches(
      {super.key, required this.enabled, required this.onPick});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final names = [
      l10n.gradeRed,
      l10n.gradeYellow,
      l10n.gradeGreen,
      l10n.gradeCyan,
      l10n.gradeBlue,
      l10n.gradeMagenta,
    ];
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        for (var i = 0; i < 6; i++)
          Padding(
            padding: const EdgeInsets.only(right: 4),
            child: LumitTooltip(
              message: names[i],
              child: GestureDetector(
                key: ValueKey<String>('grade-vector-$i'),
                behavior: HitTestBehavior.opaque,
                onTap: enabled ? () => onPick(gradeVectorHues[i]) : null,
                child: HoverMark(
                  builder: (context, over) => Container(
                    width: 14,
                    height: 14,
                    decoration: BoxDecoration(
                      color: enabled
                          ? hsvToRgb(i * 60.0, 0.6, 0.85)
                          : hsvToRgb(0, 0, 0.42),
                      borderRadius:
                          BorderRadius.circular(t.tokens.actionRadius / 2),
                      border: Border.all(
                          color: Color.lerp(t.hairline, t.textPrimary, over)!,
                          width: 1),
                    ),
                  ),
                ),
              ),
            ),
          ),
      ],
    );
  }
}
