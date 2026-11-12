// One of the qualifier's range bars: a strip of what it keys on (the hue
// circle, a saturation or luma ramp, one channel) with the range drawn over
// it.
//
// In plain terms: the box over the strip is what the qualifier takes in. Its
// top corners move one edge each, the handles under it widen or narrow the
// fade past the edges, and a drag inside the box moves the whole range. On
// the hue circle the range may run off one end and in at the other.

import 'package:flutter/gestures.dart' show DragStartBehavior;
import 'package:flutter/services.dart' show HardwareKeyboard;
import 'package:flutter/widgets.dart';

import '../widgets/controls.dart';

/// A range on a 0 to 1 bar: its two edges and how far the fade reaches past
/// each. A hue range's edges may run past either end; the rest stay inside.
typedef GradeRange = ({double low, double high, double soft});

class GradeRangeBar extends StatefulWidget {
  final GradeRange range;

  /// The hue circle: the bar wraps.
  final bool periodic;

  /// The strip under the range, as a gradient across the bar.
  final List<Color> colours;
  final List<double>? stops;

  final bool enabled;

  /// Whether the qualifier reads this bar at all. Off, the range is drawn
  /// faint.
  final bool on;
  final bool keyed;

  final VoidCallback onStart;
  final ValueChanged<GradeRange> onLive;
  final ValueChanged<GradeRange> onCommit;
  final VoidCallback onCancel;

  const GradeRangeBar({
    super.key,
    required this.range,
    required this.colours,
    required this.enabled,
    required this.onStart,
    required this.onLive,
    required this.onCommit,
    required this.onCancel,
    this.stops,
    this.periodic = false,
    this.on = true,
    this.keyed = false,
  });

  static const double height = 30;

  @override
  State<GradeRangeBar> createState() => _GradeRangeBarState();
}

enum _Grip { low, high, soft, move }

class _GradeRangeBarState extends State<GradeRangeBar> {
  GradeRange? _live;
  _Grip _grip = _Grip.move;
  double _width = 1;

  /// How close to a handle a press must land to take it.
  static const double _reach = 8;
  static const double _strip = 10;
  static const double _top = 9;

  GradeRange get _range => _live ?? widget.range;

  double get _rate => HardwareKeyboard.instance.isShiftPressed ? 0.1 : 1;

  void _start(Offset at) {
    final r = _range;
    final x = at.dx / _width;
    final lowX = _wrapNear(r.low, x), highX = _wrapNear(r.high, x);
    final reach = _reach / _width;
    final below = at.dy > _top + _strip;
    final dLow = (x - lowX).abs(), dHigh = (x - highX).abs();
    final dSoftLow = (x - _wrapNear(r.low - r.soft, x)).abs();
    final dSoftHigh = (x - _wrapNear(r.high + r.soft, x)).abs();
    if (below && (dSoftLow < reach || dSoftHigh < reach)) {
      _grip = _Grip.soft;
    } else if (dLow < reach && dLow <= dHigh) {
      _grip = _Grip.low;
    } else if (dHigh < reach) {
      _grip = _Grip.high;
    } else {
      _grip = _Grip.move;
    }
    _live = r;
    widget.onStart();
  }

  /// [v], moved a whole turn at a time on the hue circle until it is the
  /// copy nearest [x].
  double _wrapNear(double v, double x) {
    if (!widget.periodic) return v;
    var out = v;
    while (out - x > 0.5) {
      out -= 1;
    }
    while (x - out > 0.5) {
      out += 1;
    }
    return out;
  }

  void _move(double dx) {
    final r = _live;
    if (r == null) return;
    final by = dx / _width * _rate;
    final next = switch (_grip) {
      _Grip.low => (
          low: _clampEdge((r.low + by).clamp(r.high - 1, r.high).toDouble()),
          high: r.high,
          soft: r.soft
        ),
      _Grip.high => (
          low: r.low,
          high: _clampEdge((r.high + by).clamp(r.low, r.low + 1).toDouble()),
          soft: r.soft
        ),
      _Grip.soft => (
          low: r.low,
          high: r.high,
          soft: (r.soft + (_softSide(r) ? -by : by)).clamp(0.0, 1.0).toDouble()
        ),
      _Grip.move => _shift(r, by),
    };
    setState(() => _live = next);
    widget.onLive(next);
  }

  /// Which side's soft handle was taken: the low one widens leftwards.
  bool _softSide(GradeRange r) => _lastX != null && _lastX! < (r.low + r.high) / 2;
  double? _lastX;

  double _clampEdge(double v) => widget.periodic ? v : v.clamp(0.0, 1.0);

  GradeRange _shift(GradeRange r, double by) {
    if (widget.periodic) {
      return (low: r.low + by, high: r.high + by, soft: r.soft);
    }
    final span = r.high - r.low;
    final low = (r.low + by).clamp(0.0, 1.0 - span).toDouble();
    return (low: low, high: low + span, soft: r.soft);
  }

  void _end() {
    final r = _live;
    if (r == null) return;
    setState(() => _live = null);
    widget.onCommit(r);
  }

  void _cancel() {
    if (_live == null) return;
    setState(() => _live = null);
    widget.onCancel();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LayoutBuilder(builder: (context, box) {
      _width = box.maxWidth <= 0 ? 1 : box.maxWidth;
      final bar = CustomPaint(
        size: Size(box.maxWidth, GradeRangeBar.height),
        painter: _RangePainter(
          range: _range,
          periodic: widget.periodic,
          colours: widget.colours,
          stops: widget.stops,
          stripTop: _top,
          stripHeight: _strip,
          edge: !widget.enabled
              ? t.textDisabled
              : widget.keyed
                  ? t.animated
                  : _live != null
                      ? t.accent
                      : t.textPrimary,
          soft: t.textMuted,
          ground: t.surface0,
          faint: !widget.on || !widget.enabled,
        ),
      );
      if (!widget.enabled) return bar;
      return GestureDetector(
        behavior: HitTestBehavior.opaque,
        dragStartBehavior: DragStartBehavior.down,
        onHorizontalDragStart: (d) {
          _lastX = d.localPosition.dx / _width;
          _start(d.localPosition);
        },
        onHorizontalDragUpdate: (d) => _move(d.delta.dx),
        onHorizontalDragEnd: (_) => _end(),
        onHorizontalDragCancel: _cancel,
        child: MouseRegion(cursor: SystemMouseCursors.resizeLeftRight, child: bar),
      );
    });
  }
}

class _RangePainter extends CustomPainter {
  final GradeRange range;
  final bool periodic;
  final List<Color> colours;
  final List<double>? stops;
  final double stripTop, stripHeight;
  final Color edge, soft, ground;
  final bool faint;

  const _RangePainter({
    required this.range,
    required this.periodic,
    required this.colours,
    required this.stops,
    required this.stripTop,
    required this.stripHeight,
    required this.edge,
    required this.soft,
    required this.ground,
    required this.faint,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final w = size.width;
    final strip = Rect.fromLTWH(0, stripTop, w, stripHeight);
    canvas.drawRect(
      strip,
      Paint()
        ..shader = LinearGradient(colors: colours, stops: stops)
            .createShader(strip),
    );
    // What the range leaves out is dimmed, so the part keyed stands up.
    final dim = Paint()..color = ground.withValues(alpha: faint ? 0.3 : 0.6);
    void span(double a, double b, void Function(double x0, double x1) draw) {
      if (!periodic) {
        draw(a.clamp(0.0, 1.0) * w, b.clamp(0.0, 1.0) * w);
        return;
      }
      // Drawn once a turn, so a range over the seam shows at both ends.
      for (final k in const [-1.0, 0.0, 1.0]) {
        final x0 = (a + k) * w, x1 = (b + k) * w;
        if (x1 > 0 && x0 < w) draw(x0.clamp(0.0, w), x1.clamp(0.0, w));
      }
    }

    final lo = range.low, hi = range.high;
    if (periodic) {
      span(hi, lo + 1, (x0, x1) {
        canvas.drawRect(Rect.fromLTRB(x0, stripTop, x1, stripTop + stripHeight), dim);
      });
    } else {
      canvas.drawRect(
          Rect.fromLTRB(0, stripTop, lo.clamp(0.0, 1.0) * w, stripTop + stripHeight),
          dim);
      canvas.drawRect(
          Rect.fromLTRB(hi.clamp(0.0, 1.0) * w, stripTop, w, stripTop + stripHeight),
          dim);
    }
    final line = Paint()
      ..color = faint ? edge.withValues(alpha: 0.4) : edge
      ..strokeWidth = 1.5
      ..style = PaintingStyle.stroke;
    span(lo, hi, (x0, x1) {
      canvas.drawRect(
          Rect.fromLTRB(x0, stripTop - 1, x1, stripTop + stripHeight + 1), line);
    });
    // The edge handles above, the softness reach below.
    final grip = Paint()..color = faint ? edge.withValues(alpha: 0.4) : edge;
    final reach = Paint()
      ..color = soft
      ..strokeWidth = 1;
    final below = stripTop + stripHeight + 3;
    for (final k in periodic ? const [-1.0, 0.0, 1.0] : const [0.0]) {
      for (final x in [lo + k, hi + k]) {
        if (x < -0.01 || x > 1.01) continue;
        final px = x * w;
        canvas.drawPath(
          Path()
            ..moveTo(px - 4, stripTop - 7)
            ..lineTo(px + 4, stripTop - 7)
            ..lineTo(px, stripTop - 2)
            ..close(),
          grip,
        );
      }
      for (final (from, to) in [(lo + k, lo - range.soft + k), (hi + k, hi + range.soft + k)]) {
        if (to < -0.01 || to > 1.01) continue;
        final a = Offset(from * w, below), b = Offset(to * w, below);
        canvas.drawLine(a, b, reach);
        canvas.drawCircle(b, 2.5, grip);
      }
    }
  }

  @override
  bool shouldRepaint(_RangePainter old) =>
      old.range != range ||
      old.edge != edge ||
      old.faint != faint ||
      old.colours != colours;
}
