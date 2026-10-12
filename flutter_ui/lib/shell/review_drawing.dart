// What a reviewer drew over a frame, shown over that frame.
//
// A comment's drawing is strokes whose every point is from 0 to 1 of the
// picture. This lays them over the composition's own frame, fitted whole
// inside whatever box it is given, and draws them as the page in the browser
// does: the same shapes, at the same share of the picture's width.

import 'dart:math' as math;
import 'dart:ui' as ui;

import 'package:flutter/widgets.dart';

import '../state/reviews.dart';
import '../widgets/controls.dart';

/// [strokes] over [frame], which is the picture they were drawn on, or over
/// the plain surface while there is none to show. [aspect] is the picture's
/// width over its height, and [pin] the colour a pin is drawn in, which is
/// its writer's and not the stroke's own.
class ReviewDrawing extends StatelessWidget {
  final List<ReviewStroke> strokes;
  final ui.Image? frame;
  final double aspect;
  final Color pin;

  const ReviewDrawing({
    super.key,
    required this.strokes,
    required this.frame,
    required this.aspect,
    required this.pin,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return ClipRRect(
      borderRadius: BorderRadius.circular(t.tokens.controlRadius),
      child: CustomPaint(
        painter: _DrawingPainter(
          strokes: strokes,
          frame: frame,
          aspect: aspect <= 0 ? 16 / 9 : aspect,
          pin: pin,
          ground: t.surface0,
          rim: t.surface0,
        ),
        foregroundPainter: _EdgePainter(t.hairline, t.tokens.controlRadius),
      ),
    );
  }
}

/// The hairline round the box, over the picture and not beside it.
class _EdgePainter extends CustomPainter {
  final Color colour;
  final double radius;
  const _EdgePainter(this.colour, this.radius);

  @override
  void paint(Canvas canvas, Size size) => canvas.drawRRect(
        RRect.fromRectAndRadius(
            (Offset.zero & size).deflate(0.5), Radius.circular(radius)),
        Paint()
          ..style = PaintingStyle.stroke
          ..color = colour,
      );

  @override
  bool shouldRepaint(_EdgePainter old) =>
      old.colour != colour || old.radius != radius;
}

class _DrawingPainter extends CustomPainter {
  final List<ReviewStroke> strokes;
  final ui.Image? frame;
  final double aspect;
  final Color pin, ground, rim;

  const _DrawingPainter({
    required this.strokes,
    required this.frame,
    required this.aspect,
    required this.pin,
    required this.ground,
    required this.rim,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final box = Offset.zero & size;
    canvas.drawRect(box, Paint()..color = ground);
    // The picture, whole, as large as the box lets it be, and in its middle.
    final wide = math.min(size.width, size.height * aspect);
    final picture = Rect.fromCenter(
        center: box.center, width: wide, height: wide / aspect);
    final frame = this.frame;
    if (frame != null) {
      canvas.drawImageRect(
        frame,
        Rect.fromLTWH(0, 0, frame.width.toDouble(), frame.height.toDouble()),
        picture,
        Paint()..filterQuality = FilterQuality.medium,
      );
    }
    Offset at(Offset p) => Offset(picture.left + p.dx * picture.width,
        picture.top + p.dy * picture.height);
    canvas.save();
    canvas.clipRect(picture);
    for (final stroke in strokes) {
      if (stroke.shape == 'pin') continue;
      // A share of the picture's width, and never too fine to see.
      final width =
          math.max(0.0015, stroke.width) * picture.width;
      final pen = Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = width
        ..strokeCap = StrokeCap.round
        ..strokeJoin = StrokeJoin.round
        ..color = Color.fromARGB(255, stroke.colour >> 16 & 0xff,
            stroke.colour >> 8 & 0xff, stroke.colour & 0xff);
      final points = [for (final p in stroke.points) at(p)];
      final a = points.first, b = points.last;
      final path = Path();
      switch (stroke.shape) {
        case 'box':
          path.addRect(Rect.fromPoints(a, b));
        case 'line' || 'arrow':
          path
            ..moveTo(a.dx, a.dy)
            ..lineTo(b.dx, b.dy);
          if (stroke.shape == 'arrow') {
            final angle = math.atan2(b.dy - a.dy, b.dx - a.dx);
            Offset wing(double turn) => b -
                Offset(math.cos(angle + turn), math.sin(angle + turn)) *
                    (width * 5);
            final left = wing(0.5), right = wing(-0.5);
            path
              ..moveTo(left.dx, left.dy)
              ..lineTo(b.dx, b.dy)
              ..lineTo(right.dx, right.dy);
          }
        default:
          // A pen line bends at its points and runs through the middle of
          // each pair, which takes the corners off a shaky hand.
          path.moveTo(a.dx, a.dy);
          for (var i = 1; i < points.length - 1; i++) {
            final mid = (points[i] + points[i + 1]) / 2;
            path.quadraticBezierTo(
                points[i].dx, points[i].dy, mid.dx, mid.dy);
          }
          path.lineTo(b.dx, b.dy);
      }
      canvas.drawPath(path, pen);
    }
    canvas.restore();
    // Pins last and unclipped, so one near an edge is still whole. Its tip
    // is on the very point.
    for (final stroke in strokes) {
      if (stroke.shape != 'pin') continue;
      final p = at(stroke.points.first);
      const r = 7.0;
      final up = p.dy - 1.7 * r;
      final marker = Path()
        ..moveTo(p.dx, p.dy)
        ..cubicTo(p.dx - 0.45 * r, p.dy - 0.75 * r, p.dx - r, up + 0.6 * r,
            p.dx - r, up)
        ..arcToPoint(Offset(p.dx + r, up),
            radius: const Radius.circular(r), largeArc: true)
        ..cubicTo(p.dx + r, up + 0.6 * r, p.dx + 0.45 * r, p.dy - 0.75 * r,
            p.dx, p.dy)
        ..close();
      canvas
        ..drawPath(marker, Paint()..color = pin)
        ..drawPath(
            marker,
            Paint()
              ..style = PaintingStyle.stroke
              ..strokeWidth = 1.5
              ..color = rim)
        ..drawCircle(Offset(p.dx, up), r * 0.32, Paint()..color = rim);
    }
  }

  @override
  bool shouldRepaint(_DrawingPainter old) =>
      old.strokes != strokes ||
      old.frame != frame ||
      old.aspect != aspect ||
      old.pin != pin ||
      old.ground != ground;
}
