// A colour wheel for the Grade panel: a hue disc with a dot on it and a
// master bar beside it, over one of Colour balance's three colours.
//
// In plain terms: the three channels of a Lift, Gamma or Gain colour are one
// number for how bright (the master, on the bar) and a direction and distance
// for which way the picture leans (the dot on the disc). Dragging the dot
// toward orange adds red and takes away blue and green by the same amount, so
// the brightness does not move; the bar moves all three together. The two
// readings are exact: the three channel angles sit 120 degrees apart, so the
// dot's position is a plain linear map of the colour and back.

import 'dart:math' as math;

import 'package:flutter/gestures.dart' show DragStartBehavior;
import 'package:flutter/services.dart' show HardwareKeyboard;
import 'package:flutter/widgets.dart';

import '../widgets/controls.dart';
import '../widgets/colour_picker.dart' show hsvToRgb;

/// Where red, green and blue point on the disc, in degrees anticlockwise from
/// the right: the vectorscope's red, with the other two spaced evenly so the
/// three cancel and the disc reads as pure colour.
const List<double> _angles = [103, 223, 343];

/// The chroma a colour carries: how far from grey, and which way, as an (x, y)
/// in the plane of the disc with y up. The mean of the three is the master,
/// and this is what is left once it is taken out.
Offset wheelChroma(List<double> rgb) {
  final m = (rgb[0] + rgb[1] + rgb[2]) / 3;
  var x = 0.0, y = 0.0;
  for (var i = 0; i < 3; i++) {
    final a = _angles[i] * math.pi / 180;
    x += (rgb[i] - m) * math.cos(a);
    y += (rgb[i] - m) * math.sin(a);
  }
  return Offset(x * 2 / 3, y * 2 / 3);
}

/// The three channels a master and a chroma make, before the parameter's
/// own range clamps them.
List<double> wheelRgb(double master, Offset chroma) => [
      for (final deg in _angles)
        master +
            chroma.dx * math.cos(deg * math.pi / 180) +
            chroma.dy * math.sin(deg * math.pi / 180),
    ];

/// The master of a colour: its mean channel.
double wheelMaster(List<double> rgb) => (rgb[0] + rgb[1] + rgb[2]) / 3;

/// One wheel: the disc, its dot, and the master bar at its right.
///
/// A drag on the disc is relative — the dot moves with the pointer at half
/// its speed, a tenth with Shift — so a precise lean is a matter of moving
/// slowly, not of hitting a pixel. A double-click puts the dot back in the
/// middle. The bar works the same way up and down.
class GradeWheel extends StatefulWidget {
  /// The three channels as the document holds them, at the playhead.
  final List<double> rgb;

  /// What the colour is at rest: the master every channel sits at.
  final double neutral;

  /// How much chroma the disc's rim stands for.
  final double reach;

  /// The master bar's travel, either side of [neutral].
  final double masterSpan;

  /// The disc's diameter in logical pixels.
  final double size;

  /// Whether the master bar is drawn. A colour whose brightness is kept
  /// whatever its channels do has no master to move.
  final bool bar;
  final bool enabled;

  /// Whether any channel has keyframes, so the dot wears the animated colour.
  final bool keyed;

  /// The start of a gesture, before anything moves.
  final VoidCallback onStart;

  /// A drag tick, with the three channels to preview.
  final ValueChanged<List<double>> onLive;

  /// The end of a gesture, with the three channels to commit.
  final ValueChanged<List<double>> onCommit;

  /// A gesture that began and wrote nothing.
  final VoidCallback onCancel;

  const GradeWheel({
    super.key,
    required this.rgb,
    required this.neutral,
    required this.reach,
    required this.masterSpan,
    required this.size,
    this.bar = true,
    required this.enabled,
    required this.keyed,
    required this.onStart,
    required this.onLive,
    required this.onCommit,
    required this.onCancel,
  });

  @override
  State<GradeWheel> createState() => _GradeWheelState();
}

class _GradeWheelState extends State<GradeWheel> {
  /// The colour in hand while a drag is in flight, as master and chroma.
  double? _master;
  Offset? _chroma;
  bool _onBar = false;

  double get _liveMaster => _master ?? wheelMaster(widget.rgb);
  Offset get _liveChroma => _chroma ?? wheelChroma(widget.rgb);

  /// Half speed, a tenth with Shift: the disc is a control, not a target.
  double get _rate => HardwareKeyboard.instance.isShiftPressed ? 0.1 : 0.5;

  void _start(bool bar) {
    _onBar = bar;
    _master = wheelMaster(widget.rgb);
    _chroma = wheelChroma(widget.rgb);
    widget.onStart();
  }

  void _move(Offset delta) {
    final radius = widget.size / 2;
    if (_onBar) {
      final per = widget.masterSpan * 2 / widget.size;
      _master = (_liveMaster - delta.dy * per * _rate).clamp(
          widget.neutral - widget.masterSpan,
          widget.neutral + widget.masterSpan);
    } else {
      final per = widget.reach / radius;
      var next = _liveChroma + Offset(delta.dx, -delta.dy) * per * _rate;
      if (next.distance > widget.reach) {
        next = next / next.distance * widget.reach;
      }
      _chroma = next;
    }
    setState(() {});
    widget.onLive(wheelRgb(_liveMaster, _liveChroma));
  }

  void _end() {
    final rgb = wheelRgb(_liveMaster, _liveChroma);
    setState(() {
      _master = null;
      _chroma = null;
    });
    widget.onCommit(rgb);
  }

  /// A recogniser that lost the arena: the other axis, or the panel's
  /// scroll, has the pointer. Nothing is written, so one drag stays one step.
  /// The loser rarely got as far as [_start]; when it did, the gesture it
  /// opened is closed with nothing in it.
  void _cancel() {
    if (_master == null) return;
    setState(() {
      _master = null;
      _chroma = null;
    });
    widget.onCancel();
  }

  /// Double-clicks. Already there is nothing to write, so no step is taken
  /// and no effect is added for it.
  void _resetChroma() {
    if (wheelChroma(widget.rgb).distance < 1e-6) return;
    widget.onStart();
    widget.onCommit(wheelRgb(wheelMaster(widget.rgb), Offset.zero));
  }

  void _resetMaster() {
    if ((wheelMaster(widget.rgb) - widget.neutral).abs() < 1e-6) return;
    widget.onStart();
    widget.onCommit(wheelRgb(widget.neutral, wheelChroma(widget.rgb)));
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final size = widget.size;
    final chroma = _liveChroma;
    final master = _liveMaster;
    final leaning = chroma.distance > 1e-6;
    final lifted = (master - widget.neutral).abs() > 1e-6;
    final dot =
        widget.keyed ? t.animated : (leaning ? t.accent : t.textPrimary);
    final knob =
        widget.keyed ? t.animated : (lifted ? t.accent : t.textPrimary);
    return Row(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.center,
      children: [
        _drag(
          bar: false,
          onDouble: _resetChroma,
          child: CustomPaint(
            size: Size(size, size),
            painter: _WheelPainter(
              chroma: chroma / widget.reach,
              dot: dot,
              ring: t.hairlineStrong,
              cross: t.hairline,
              centre: t.surface1,
              ground: t.surface0,
              enabled: widget.enabled,
            ),
          ),
        ),
        if (widget.bar) const SizedBox(width: 6),
        if (widget.bar)
          _drag(
            bar: true,
            onDouble: _resetMaster,
            child: CustomPaint(
              size: Size(10, size),
              painter: _BarPainter(
                at: ((master - widget.neutral) / widget.masterSpan)
                    .clamp(-1.0, 1.0),
                knob: knob,
                track: t.hairlineStrong,
                mark: t.hairline,
              ),
            ),
          ),
      ],
    );
  }

  /// The disc and the bar take the same gesture: a relative drag on one axis
  /// pair, a double-click to reset. Vertical and horizontal recognisers
  /// rather than a pan, so the panel's scroll does not win every upward
  /// drag (the curve editor's reasoning).
  Widget _drag(
      {required bool bar,
      required VoidCallback onDouble,
      required Widget child}) {
    if (!widget.enabled) return child;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      dragStartBehavior: DragStartBehavior.down,
      onDoubleTap: onDouble,
      onVerticalDragStart: (_) => _start(bar),
      onVerticalDragUpdate: (d) => _move(d.delta),
      onVerticalDragEnd: (_) => _end(),
      onVerticalDragCancel: _cancel,
      onHorizontalDragStart: bar ? null : (_) => _start(bar),
      onHorizontalDragUpdate: bar ? null : (d) => _move(d.delta),
      onHorizontalDragEnd: bar ? null : (_) => _end(),
      onHorizontalDragCancel: bar ? null : _cancel,
      child: MouseRegion(cursor: SystemMouseCursors.precise, child: child),
    );
  }
}

/// The disc: hues round the rim fading to the panel's grey in the middle,
/// a faint cross, and the dot.
class _WheelPainter extends CustomPainter {
  /// The dot, as a fraction of the radius with y up.
  final Offset chroma;
  final Color dot, ring, cross, centre, ground;
  final bool enabled;

  const _WheelPainter({
    required this.chroma,
    required this.dot,
    required this.ring,
    required this.cross,
    required this.centre,
    required this.ground,
    required this.enabled,
  });

  /// A strong colour at [hue] degrees, so the rim reads clearly; the grey
  /// centre and the dark ring round the dot keep the dot the thing to look
  /// at. Disabled, the wheel is the same shape in grey.
  Color _hue(double hue) =>
      enabled ? hsvToRgb(hue, 0.95, 0.95) : hsvToRgb(hue, 0.0, 0.42);

  @override
  void paint(Canvas canvas, Size size) {
    final c = Offset(size.width / 2, size.height / 2);
    final r = size.shortestSide / 2;
    final rect = Rect.fromCircle(center: c, radius: r);
    // The sweep runs clockwise from three o'clock and our angles
    // anticlockwise, so the hue at each step counts down from red's angle. It
    // starts at zero and runs the full circle: a sweep doesn't wrap below
    // zero, so starting it at red's angle left a hard seam at three o'clock.
    const steps = 24;
    canvas.drawCircle(
      c,
      r,
      Paint()
        ..shader = SweepGradient(
          colors: [
            for (var i = 0; i <= steps; i++)
              _hue(((-i * 360 / steps - _angles[0]) % 360 + 360) % 360)
          ],
        ).createShader(rect),
    );
    // Grey in the middle: the centre is no lean at all.
    canvas.drawCircle(
      c,
      r,
      Paint()
        ..shader = RadialGradient(
          colors: [centre, centre.withValues(alpha: 0)],
          // Gone by three quarters out, so the rim keeps its full colour.
          stops: const [0.08, 0.75],
        ).createShader(rect),
    );
    final hair = Paint()
      ..color = cross
      ..strokeWidth = 1;
    canvas.drawLine(Offset(c.dx - r, c.dy), Offset(c.dx + r, c.dy), hair);
    canvas.drawLine(Offset(c.dx, c.dy - r), Offset(c.dx, c.dy + r), hair);
    canvas.drawCircle(
      c,
      r - 0.5,
      Paint()
        ..color = ring
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
    final at = c + Offset(chroma.dx * r, -chroma.dy * r);
    canvas.drawCircle(at, 5, Paint()..color = ground);
    canvas.drawCircle(at, 3.5, Paint()..color = dot);
  }

  @override
  bool shouldRepaint(_WheelPainter old) =>
      old.chroma != chroma ||
      old.dot != dot ||
      old.ring != ring ||
      old.enabled != enabled ||
      old.centre != centre;
}

/// The master bar: a track with a mark at the neutral point and a knob.
class _BarPainter extends CustomPainter {
  /// The knob, -1 at the bottom to 1 at the top.
  final double at;
  final Color knob, track, mark;

  const _BarPainter({
    required this.at,
    required this.knob,
    required this.track,
    required this.mark,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final x = size.width / 2;
    final top = 4.0, bottom = size.height - 4;
    canvas.drawLine(
      Offset(x, top),
      Offset(x, bottom),
      Paint()
        ..color = track
        ..strokeWidth = 1,
    );
    final mid = (top + bottom) / 2;
    canvas.drawLine(
      Offset(x - 3, mid),
      Offset(x + 3, mid),
      Paint()
        ..color = mark
        ..strokeWidth = 1,
    );
    final y = mid - at * (bottom - top) / 2;
    canvas.drawRRect(
      RRect.fromRectAndRadius(
          Rect.fromCenter(center: Offset(x, y), width: 8, height: 4),
          const Radius.circular(2)),
      Paint()..color = knob,
    );
  }

  @override
  bool shouldRepaint(_BarPainter old) =>
      old.at != at || old.knob != knob || old.track != track;
}
