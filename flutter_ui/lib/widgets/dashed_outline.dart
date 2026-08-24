// A dashed border drawn around whatever it wraps.
//
// **In plain terms.** Something switched off should still be readable. Dimming
// it — dropping the whole thing to 40% — makes the words hard to read at the
// moment you most want to check what you turned off. So the state is shown by a
// dashed line around the thing instead, and nothing inside changes colour
// (docs/15 §5: "a bypassed effect draws as a dashed outline, not a dimmed row").
//
// The dashes are cut out of the rounded rectangle's own path, so the corners
// stay round in Round mode and square in Sharp without the painter knowing
// which mode it is in.

import 'package:flutter/widgets.dart';

import 'controls.dart' show ThemeScope;

/// Wraps [child] in a 1px dashed border in `hairlineStrong`, cornered to the
/// theme's control radius.
class DashedOutline extends StatelessWidget {
  final Widget child;

  const DashedOutline({super.key, required this.child});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return CustomPaint(
      foregroundPainter: _DashedBorderPainter(
        colour: t.hairlineStrong,
        radius: t.tokens.controlRadius,
      ),
      child: child,
    );
  }
}

class _DashedBorderPainter extends CustomPainter {
  final Color colour;
  final double radius;

  const _DashedBorderPainter({required this.colour, required this.radius});

  /// Dash 3, gap 2 — short enough to read as dashed on a heading bar's width,
  /// long enough not to blur into a solid line at 100% scaling.
  static const double _dash = 3;
  static const double _gap = 2;

  @override
  void paint(Canvas canvas, Size size) {
    if (size.isEmpty) return;
    // Inset by half the stroke so the line lands inside the box rather than
    // straddling its edge, and clamp the corner: Round's radius is a stadium
    // sentinel far larger than any heading is tall.
    final rect = Offset.zero & size;
    final corner = radius.clamp(0.0, size.shortestSide / 2).toDouble();
    final path = Path()
      ..addRRect(RRect.fromRectAndRadius(
        rect.deflate(0.5),
        Radius.circular(corner),
      ));

    final paint = Paint()
      ..color = colour
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;

    for (final metric in path.computeMetrics()) {
      var at = 0.0;
      while (at < metric.length) {
        canvas.drawPath(
          metric.extractPath(at, (at + _dash).clamp(0.0, metric.length)),
          paint,
        );
        at += _dash + _gap;
      }
    }
  }

  @override
  bool shouldRepaint(_DashedBorderPainter old) =>
      old.colour != colour || old.radius != radius;
}
