// The easing editor's drawings: the box the shape is bent in, the strip that
// plays it, and the library's thumbnails (see `easing_editor.dart`).

import 'package:flutter/widgets.dart';

import '../theme/theme.dart';
import 'easing_curve.dart';

class EasingBoxPainter extends CustomPainter {
  final EasingCurve curve;
  final Rect box;
  final LumitTheme theme;

  /// The handle under the pointer or in hand, drawn larger.
  final int? lit;

  /// Where the preview is in time, while it plays.
  final double? playhead;

  const EasingBoxPainter({
    required this.curve,
    required this.box,
    required this.theme,
    required this.lit,
    required this.playhead,
  });

  Offset _at(double x, double y) =>
      Offset(box.left + x * box.width, box.bottom - y * box.height);

  @override
  void paint(Canvas canvas, Size size) {
    final t = theme;
    canvas.drawRRect(
      RRect.fromRectAndRadius(
          Offset.zero & size, Radius.circular(t.tokens.wellRadius)),
      Paint()..color = t.surface0,
    );

    // The grid: quarters faint, the start and end values across the whole
    // width so the overshoot room reads as part of the graph.
    final faint = Paint()
      ..color = t.hairline.withValues(alpha: t.hairline.a * 0.5)
      ..strokeWidth = 1;
    for (final q in const [0.25, 0.5, 0.75]) {
      canvas.drawLine(
          _at(q, -easingHandleReach), _at(q, 1 + easingHandleReach), faint);
      canvas.drawLine(_at(0, q), _at(1, q), faint);
    }
    final line = Paint()
      ..color = t.hairline
      ..strokeWidth = 1;
    canvas.drawLine(
        Offset(0, box.bottom), Offset(size.width, box.bottom), line);
    canvas.drawLine(Offset(0, box.top), Offset(size.width, box.top), line);
    canvas.drawLine(
        _at(0, -easingHandleReach), _at(0, 1 + easingHandleReach), line);
    canvas.drawLine(
        _at(1, -easingHandleReach), _at(1, 1 + easingHandleReach), line);

    // The chord: no ease at all, what the shape is read against.
    _dashed(canvas, _at(0, 0), _at(1, 1), line);

    final p1 = _at(curve.x1, curve.y1);
    final p2 = _at(curve.x2, curve.y2);
    final stem = Paint()
      ..color = t.textMuted
      ..strokeWidth = 1;
    canvas.drawLine(_at(0, 0), p1, stem);
    canvas.drawLine(_at(1, 1), p2, stem);

    final path = Path()..moveTo(_at(0, 0).dx, _at(0, 0).dy);
    const steps = 64;
    for (var i = 1; i <= steps; i++) {
      final u = i / steps;
      final p = _at(curve.xAt(u), curve.yAt(u));
      path.lineTo(p.dx, p.dy);
    }
    canvas.drawPath(
      path,
      Paint()
        ..color = t.curve.first
        ..strokeWidth = 2
        ..style = PaintingStyle.stroke
        ..strokeCap = StrokeCap.round,
    );

    final time = playhead;
    if (time != null) {
      canvas.drawLine(_at(time, -easingHandleReach),
          _at(time, 1 + easingHandleReach), faint);
      canvas.drawCircle(
          _at(time, curve.valueAt(time)), 3.5, Paint()..color = t.textPrimary);
    }

    // The two ends are keys: small diamonds, as the Timeline draws them.
    final end = Paint()..color = t.textSecondary;
    for (final p in [_at(0, 0), _at(1, 1)]) {
      canvas.drawPath(
          Path()
            ..moveTo(p.dx, p.dy - 4)
            ..lineTo(p.dx + 4, p.dy)
            ..lineTo(p.dx, p.dy + 4)
            ..lineTo(p.dx - 4, p.dy)
            ..close(),
          end);
    }

    for (final (handle, p) in [(1, p1), (2, p2)]) {
      final r = lit == handle ? 6.0 : 4.5;
      canvas.drawCircle(p, r + 1.5, Paint()..color = t.surface0);
      canvas.drawCircle(p, r, Paint()..color = t.accent);
    }
  }

  static void _dashed(Canvas canvas, Offset a, Offset b, Paint paint) {
    final d = b - a;
    final n = (d.distance / 6).floor();
    for (var i = 0; i < n; i += 2) {
      canvas.drawLine(a + d * (i / n), a + d * ((i + 1) / n), paint);
    }
  }

  @override
  bool shouldRepaint(EasingBoxPainter old) =>
      old.curve != curve ||
      old.box != box ||
      old.theme != theme ||
      old.lit != lit ||
      old.playhead != playhead;
}

/// The strip under the box: a dot that moves with the shape, and a faint one
/// that keeps plain time beside it.
class EasingPreviewPainter extends CustomPainter {
  final EasingCurve curve;
  final Rect box;
  final LumitTheme theme;
  final double time;

  const EasingPreviewPainter({
    required this.curve,
    required this.box,
    required this.theme,
    required this.time,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final y = size.height / 2;
    double at(double v) => box.left + v * box.width;
    final line = Paint()
      ..color = theme.hairline
      ..strokeWidth = 1;
    canvas.drawLine(Offset(at(0), y), Offset(at(1), y), line);
    for (final end in [at(0), at(1)]) {
      canvas.drawLine(Offset(end, y - 3), Offset(end, y + 3), line);
    }
    if (time > 0 && time < 1) {
      canvas.drawCircle(
          Offset(at(time), y), 3, Paint()..color = theme.textDisabled);
    }
    final x = at(curve.valueAt(time)).clamp(3.0, size.width - 3).toDouble();
    canvas.drawCircle(Offset(x, y), 4.5, Paint()..color = theme.accent);
  }

  @override
  bool shouldRepaint(EasingPreviewPainter old) =>
      old.curve != curve ||
      old.box != box ||
      old.theme != theme ||
      old.time != time;
}

/// A tile's thumbnail: the chord as a hairline and the curve over it, walked
/// by the same [EasingCurve.xAt] and [EasingCurve.yAt] the box walks.
class EasingTilePainter extends CustomPainter {
  final EasingCurve curve;
  final LumitTheme theme;
  final bool current;

  const EasingTilePainter({
    required this.curve,
    required this.theme,
    required this.current,
  });

  @override
  void paint(Canvas canvas, Size size) {
    canvas.drawRRect(
      RRect.fromRectAndRadius(
          Offset.zero & size, Radius.circular(theme.tokens.wellRadius)),
      Paint()..color = theme.surface0,
    );
    // A graph's proportions however wide the tile grows: a stretched curve
    // reads flatter than the shape it is.
    final pad = size.height * 0.2;
    final h = size.height - 2 * pad;
    final w = (size.width * 0.76).clamp(0.0, h * 2.2).toDouble();
    final box = Rect.fromLTWH((size.width - w) / 2, pad, w, h);
    Offset at(double x, double y) =>
        Offset(box.left + x * box.width, box.bottom - y * box.height);

    canvas.drawLine(
      at(0, 0),
      at(1, 1),
      Paint()
        ..color = theme.hairline
        ..strokeWidth = 1,
    );
    final path = Path()..moveTo(at(0, 0).dx, at(0, 0).dy);
    const steps = 32;
    for (var i = 1; i <= steps; i++) {
      final u = i / steps;
      final p = at(curve.xAt(u), curve.yAt(u));
      path.lineTo(p.dx, p.dy);
    }
    canvas.drawPath(
      path,
      Paint()
        ..color = current ? theme.accent : theme.curve.first
        ..strokeWidth = 1.5
        ..style = PaintingStyle.stroke
        ..strokeCap = StrokeCap.round,
    );
  }

  @override
  bool shouldRepaint(EasingTilePainter old) =>
      old.curve != curve || old.theme != theme || old.current != current;
}
