// The Timeline's zoom as a dial: Desk's one control of its own. A disc with a
// tick ring engraved round it and a single index, turned by dragging round
// its centre. It writes the same logarithmic position the zoom slider does,
// so the two are one control drawn two ways.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';

import '../widgets/controls.dart' show ThemeScope;

/// The dial. [value] runs 0 to 1 over a three-quarter turn that starts at the
/// bottom left and ends at the bottom right, the gap at the foot being where
/// the hand rests. A drag reports live through [onChangeLive] and commits
/// through [onChanged] on release, the slider's own arrangement.
class TimelineZoomDial extends StatefulWidget {
  final double value;
  final ValueChanged<double> onChanged;
  final ValueChanged<double>? onChangeLive;
  final VoidCallback? onChangeStart;
  final VoidCallback? onChangeEnd;

  /// The disc's diameter, and the whole control's with the tick ring round
  /// it: the ring stands outside the disc, and the pair fit the 20px foot.
  static const double disc = 14;
  static const double size = 20;

  /// How many marks the ring carries, and the turn they cover.
  static const int marks = 12;
  static const double sweep = math.pi * 1.5;
  static const double startAngle = math.pi * 0.75;

  const TimelineZoomDial({
    super.key,
    required this.value,
    required this.onChanged,
    this.onChangeLive,
    this.onChangeStart,
    this.onChangeEnd,
  });

  /// Where a pointer at [at], measured from the dial's centre, puts the
  /// value: its angle along the sweep, clamped to the nearer end across the
  /// gap at the foot.
  static double valueAt(Offset at) {
    var angle = math.atan2(at.dy, at.dx) - startAngle;
    while (angle < 0) {
      angle += math.pi * 2;
    }
    if (angle <= sweep) return angle / sweep;
    // In the gap: the nearer end wins.
    return angle - sweep < (math.pi * 2 - angle) ? 1 : 0;
  }

  @override
  State<TimelineZoomDial> createState() => _TimelineZoomDialState();
}

class _TimelineZoomDialState extends State<TimelineZoomDial> {
  double? _pending;

  double _at(Offset local) => TimelineZoomDial.valueAt(local -
      const Offset(TimelineZoomDial.size / 2, TimelineZoomDial.size / 2));

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      key: const ValueKey('tl-zoom-dial'),
      behavior: HitTestBehavior.opaque,
      onPanStart: (d) {
        widget.onChangeStart?.call();
        final v = _at(d.localPosition);
        setState(() => _pending = v);
        (widget.onChangeLive ?? widget.onChanged)(v);
      },
      onPanUpdate: (d) {
        final v = _at(d.localPosition);
        setState(() => _pending = v);
        (widget.onChangeLive ?? widget.onChanged)(v);
      },
      onPanEnd: (_) {
        final v = _pending;
        setState(() => _pending = null);
        if (v != null) widget.onChanged(v);
        widget.onChangeEnd?.call();
      },
      onPanCancel: () {
        setState(() => _pending = null);
        widget.onChangeEnd?.call();
      },
      child: CustomPaint(
        size: const Size.square(TimelineZoomDial.size),
        painter: _DialPainter(
          value: _pending ?? widget.value,
          disc: t.surface1,
          rim: t.hairlineStrong,
          mark: t.textMuted,
          index: t.accent,
        ),
      ),
    );
  }
}

class _DialPainter extends CustomPainter {
  final double value;
  final Color disc, rim, mark, index;

  const _DialPainter({
    required this.value,
    required this.disc,
    required this.rim,
    required this.mark,
    required this.index,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final centre = Offset(size.width / 2, size.height / 2);
    const r = TimelineZoomDial.disc / 2;
    canvas.drawCircle(centre, r, Paint()..color = disc);
    canvas.drawCircle(
        centre,
        r - 0.5,
        Paint()
          ..color = rim
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1);
    // The engraved ring: marks just outside the rim, along the sweep.
    final marks = Paint()
      ..color = mark
      ..strokeWidth = 1;
    for (var i = 0; i < TimelineZoomDial.marks; i++) {
      final a = TimelineZoomDial.startAngle +
          TimelineZoomDial.sweep * i / (TimelineZoomDial.marks - 1);
      final dir = Offset(math.cos(a), math.sin(a));
      canvas.drawLine(
          centre + dir * (r + 1.5), centre + dir * (r + 3), marks);
    }
    // The index: one 2px accent line from near the centre to the rim.
    final a = TimelineZoomDial.startAngle +
        TimelineZoomDial.sweep * value.clamp(0.0, 1.0);
    final dir = Offset(math.cos(a), math.sin(a));
    canvas.drawLine(
        centre + dir * 2,
        centre + dir * (r - 1.5),
        Paint()
          ..color = index
          ..strokeWidth = 2
          ..strokeCap = StrokeCap.round);
  }

  @override
  bool shouldRepaint(_DialPainter old) =>
      old.value != value ||
      old.disc != disc ||
      old.rim != rim ||
      old.mark != mark ||
      old.index != index;
}
