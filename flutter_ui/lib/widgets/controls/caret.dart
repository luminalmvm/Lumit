// The small quiet chevron that says a thing opens.

import 'package:flutter/widgets.dart';

/// A dropdown's face wears one pointing down, a submenu row one pointing the
/// way its flyout goes. The border or the row already says this is a
/// control, so the mark only has to say which kind.
class Caret extends StatelessWidget {
  final Color colour;
  final AxisDirection direction;
  const Caret(
      {super.key, required this.colour, this.direction = AxisDirection.down});

  @override
  Widget build(BuildContext context) => CustomPaint(
        size: const Size(7, 7),
        painter: _CaretPainter(colour, direction),
      );
}

class _CaretPainter extends CustomPainter {
  final Color colour;
  final AxisDirection direction;
  const _CaretPainter(this.colour, this.direction);

  @override
  void paint(Canvas canvas, Size size) {
    final p = Paint()
      ..color = colour
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1.5
      ..strokeCap = StrokeCap.round;
    final w = size.width, h = size.height;
    final (a, b, c) = switch (direction) {
      AxisDirection.down => (
          Offset(w * 0.2, h * 0.35),
          Offset(w * 0.5, h * 0.65),
          Offset(w * 0.8, h * 0.35)
        ),
      AxisDirection.up => (
          Offset(w * 0.2, h * 0.65),
          Offset(w * 0.5, h * 0.35),
          Offset(w * 0.8, h * 0.65)
        ),
      AxisDirection.right => (
          Offset(w * 0.35, h * 0.2),
          Offset(w * 0.65, h * 0.5),
          Offset(w * 0.35, h * 0.8)
        ),
      AxisDirection.left => (
          Offset(w * 0.65, h * 0.2),
          Offset(w * 0.35, h * 0.5),
          Offset(w * 0.65, h * 0.8)
        ),
    };
    canvas.drawLine(a, b, p);
    canvas.drawLine(b, c, p);
  }

  @override
  bool shouldRepaint(_CaretPainter old) =>
      old.colour != colour || old.direction != direction;
}
