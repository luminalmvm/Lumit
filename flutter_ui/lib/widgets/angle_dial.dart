// The angle dial — the round control an angle parameter draws beneath its
// number (docs/07-UI-SPEC.md §6, which has named "angle dials" among the
// parameter widgets since the spec was written).
//
// **In plain terms.** Some numbers are directions: which way the iris is
// turned, which way a shake leans. Typing "90" for that works, but you cannot
// *see* 90 — so the dial draws it as a hand on a clock face and lets you grab
// the hand and swing it. The number underneath is the same value; the two are
// one control shown twice, and either can be used.
//
// **Why it winds rather than stops.** An angle is unbounded on purpose: an
// animated rotation goes 350° → 370°, not 350° → 10°, because a keyframe pair
// interpolating the second way spins backwards through the whole circle. So
// dragging past the top keeps counting, and the dial shows the remainder while
// the value keeps the turns. This mirrors how the value is stored — degrees,
// no wrapping — and is why the field beside it can read "0x+90.0°".

import 'dart:math' as math;

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import 'controls.dart';

/// A circular angle control, `size` px across.
///
/// Reports degrees measured clockwise from twelve o'clock, matching the way the
/// number beside it reads and the way the engine's rotations are authored.
class AngleDial extends StatefulWidget {
  /// The current angle in degrees. May be any magnitude — 720 is two turns, and
  /// the dial draws the remainder while the value keeps the count.
  final double degrees;

  /// Snapping increment in degrees while Shift is held.
  final double step;

  /// Whether the dial responds at all. A greyed row draws its dial faded and
  /// takes no gesture (docs/08's conditional enablement).
  final bool enabled;

  /// Live during a drag.
  final ValueChanged<double> onChanged;

  /// Once, at the end of a drag — the commit. Falls back to [onChanged].
  final ValueChanged<double>? onChangeEnd;

  final double size;

  const AngleDial({
    super.key,
    required this.degrees,
    required this.onChanged,
    this.onChangeEnd,
    this.step = 15,
    this.enabled = true,
    this.size = 34,
  });

  @override
  State<AngleDial> createState() => _AngleDialState();
}

class _AngleDialState extends State<AngleDial> {
  /// The angle the drag started from, and the pointer angle it started at —
  /// the drag applies the *difference* rather than jumping the hand to wherever
  /// the pointer went down. Grabbing the hand a little off-centre should not
  /// snap it.
  double _startValue = 0;
  double _startPointer = 0;
  bool _shift = false;

  /// Degrees clockwise from twelve o'clock for a point in the dial's box.
  double _pointerDegrees(Offset local) {
    final c = widget.size / 2;
    final v = local - Offset(c, c);
    // atan2 measures anticlockwise from three o'clock; this is clockwise from
    // twelve, which is what the number beside the dial means.
    return (math.atan2(v.dx, -v.dy) * 180 / math.pi);
  }

  void _begin(Offset local) {
    _startValue = widget.degrees;
    _startPointer = _pointerDegrees(local);
  }

  double _valueFor(Offset local) {
    // The shortest way round from where the drag started, so crossing twelve
    // o'clock counts as a small move rather than a 360° jump.
    var delta = _pointerDegrees(local) - _startPointer;
    if (delta > 180) delta -= 360;
    if (delta < -180) delta += 360;
    final raw = _startValue + delta;
    if (!_shift || widget.step <= 0) return raw;
    return (raw / widget.step).roundToDouble() * widget.step;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final dial = CustomPaint(
      size: Size.square(widget.size),
      painter: _DialPainter(
        degrees: widget.degrees,
        rim: widget.enabled ? t.hairlineStrong : t.hairline,
        hand: widget.enabled ? t.accent : t.textDisabled,
        face: t.surface2,
      ),
    );

    if (!widget.enabled) {
      return SizedBox.square(dimension: widget.size, child: dial);
    }

    return Focus(
      // Shift snaps to `step`; tracked here because a raw pointer event does
      // not carry the modifier state on every platform.
      onKeyEvent: (_, event) {
        _shift = HardwareKeyboard.instance.isShiftPressed;
        return KeyEventResult.ignored;
      },
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onPanDown: (d) {
            _shift = HardwareKeyboard.instance.isShiftPressed;
            _begin(d.localPosition);
          },
          onPanUpdate: (d) {
            _shift = HardwareKeyboard.instance.isShiftPressed;
            widget.onChanged(_valueFor(d.localPosition));
          },
          onPanEnd: (d) {
            final end = widget.onChangeEnd ?? widget.onChanged;
            end(_valueFor(d.localPosition));
          },
          child: SizedBox.square(dimension: widget.size, child: dial),
        ),
      ),
    );
  }
}

class _DialPainter extends CustomPainter {
  final double degrees;
  final Color rim;
  final Color hand;
  final Color face;

  const _DialPainter({
    required this.degrees,
    required this.rim,
    required this.hand,
    required this.face,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final c = Offset(size.width / 2, size.height / 2);
    final r = size.width / 2 - 1;

    canvas.drawCircle(c, r, Paint()..color = face);
    canvas.drawCircle(
      c,
      r,
      Paint()
        ..color = rim
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );

    // Clockwise from twelve. The value may be any magnitude; only where the
    // hand points is drawn, which is the remainder.
    final a = (degrees - 90) * math.pi / 180;
    final tip = c + Offset(math.cos(a), math.sin(a)) * r;
    canvas.drawLine(
      c,
      tip,
      Paint()
        ..color = hand
        ..strokeWidth = 1.5
        ..strokeCap = StrokeCap.round,
    );
    canvas.drawCircle(c, 1.5, Paint()..color = hand);
  }

  @override
  bool shouldRepaint(_DialPainter old) =>
      old.degrees != degrees ||
      old.hand != hand ||
      old.rim != rim ||
      old.face != face;
}

/// An angle as **turns and degrees** — the `2x +30°` pair every editor shows for
/// a rotation, drawn as two fields side by side.
///
/// **Why a rotation needs two boxes.** 30° and 390° are the same picture but not
/// the same *animation*: a key at 30 and a key at 390 travels a full turn
/// between them, and one at 30 followed by one at 30 does not move at all. A
/// single field showing "30" cannot tell you which you have, and typing 390 into
/// a box labelled degrees reads as a mistake. Splitting the turns out makes the
/// stored number legible without changing it.
///
/// **The value is still one number.** `turns · 360 + degrees` is what is written,
/// which is exactly why the split is a display concern: the property animates,
/// serialises and interpolates as the single angle it always was, and two
/// independently keyed halves could disagree about where the rotation is.
/// Typing 400 into the degrees box therefore rolls over of its own accord — the
/// value becomes 400 and redraws as `1x +40°`.
class TurnsAndDegreesField extends StatelessWidget {
  /// The whole angle, turns included.
  final double degrees;

  /// Mid-drag preview, and the committed value. `onChanged` is null on an
  /// animated property, where previewing a static value would lie about the
  /// curve.
  final ValueChanged<double>? onChanged;
  final ValueChanged<double> onCommit;

  final bool enabled;

  /// Key stem for the two fields, so a rebuild keeps the caret.
  final String keyName;

  const TurnsAndDegreesField({
    super.key,
    required this.degrees,
    required this.onCommit,
    required this.keyName,
    this.onChanged,
    this.enabled = true,
  });

  /// The whole turns in an angle, toward zero — so −370° reads as −1 turn and
  /// −10°, which is how it is spoken.
  static double turnsOf(double value) => (value / 360).truncateToDouble();

  /// What is left after the turns.
  static double degreesOf(double value) => value - turnsOf(value) * 360;

  @override
  Widget build(BuildContext context) {
    final turns = turnsOf(degrees);
    final rest = degreesOf(degrees);

    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        SizedBox(
          width: 30,
          child: DragValueField(
            key: ValueKey<String>('angle-turns-$keyName'),
            value: turns,
            min: -10000,
            max: 10000,
            speed: 1,
            decimals: 0,
            suffix: 'x',
            onChanged: (v) => onCommit(v.toDouble() * 360 + rest),
            onChangeLive: onChanged == null
                ? null
                : (v) => onChanged!(v.toDouble() * 360 + rest),
            onChangeEnd: (v) => onCommit(v.toDouble() * 360 + rest),
          ),
        ),
        const SizedBox(width: 3),
        SizedBox(
          width: 54,
          child: DragValueField(
            key: ValueKey<String>('angle-degrees-$keyName'),
            value: rest,
            // Open, not clamped to ±360: typing 400 here is a legitimate way to
            // say "one turn and forty", and it redraws as that.
            min: -1000000,
            max: 1000000,
            speed: 1,
            decimals: 1,
            suffix: '°',
            onChanged: (v) => onCommit(turns * 360 + v.toDouble()),
            onChangeLive: onChanged == null
                ? null
                : (v) => onChanged!(turns * 360 + v.toDouble()),
            onChangeEnd: (v) => onCommit(turns * 360 + v.toDouble()),
          ),
        ),
      ],
    );
  }
}
