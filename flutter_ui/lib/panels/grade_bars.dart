// A wheel's colour as four upright bars: the master, then red, green and
// blue. The Grade panel's Bars mode.
//
// In plain terms: the same three numbers a wheel holds, one bar each, with a
// fourth that moves all three together. A drag is relative, at a fifth of the
// pointer's speed and a twentieth with Shift or Ctrl, as the wheel is; a
// double-click puts one bar back.

import 'package:flutter/gestures.dart' show DragStartBehavior;
import 'package:flutter/services.dart' show HardwareKeyboard;
import 'package:flutter/widgets.dart';

import '../widgets/controls.dart';
import 'grade_wheel.dart' show wheelMaster;

class GradeBars extends StatefulWidget {
  /// The three channels as the document holds them, at the playhead.
  final List<double> rgb;

  /// Where every channel rests.
  final double neutral;

  /// How far the bars travel either side of [neutral].
  final double span;

  /// The colour's own bounds, which a channel is kept inside.
  final double min, max;

  final double height;
  final bool enabled;
  final bool keyed;

  /// The master's colour, then red's, green's and blue's.
  final List<Color> colours;

  final VoidCallback onStart;
  final ValueChanged<List<double>> onLive;
  final ValueChanged<List<double>> onCommit;
  final VoidCallback onCancel;

  const GradeBars({
    super.key,
    required this.rgb,
    required this.neutral,
    required this.span,
    required this.min,
    required this.max,
    required this.height,
    required this.enabled,
    required this.keyed,
    required this.colours,
    required this.onStart,
    required this.onLive,
    required this.onCommit,
    required this.onCancel,
  });

  @override
  State<GradeBars> createState() => _GradeBarsState();
}

class _GradeBarsState extends State<GradeBars> {
  /// The colour in hand during a drag, and which bar has it: 0 the master.
  List<double>? _live;
  int _bar = 0;

  double get _rate {
    final keys = HardwareKeyboard.instance;
    return keys.isShiftPressed || keys.isControlPressed ? 0.05 : 0.2;
  }

  List<double> get _rgb => _live ?? widget.rgb;

  void _start(int bar) {
    _bar = bar;
    _live = [...widget.rgb];
    widget.onStart();
  }

  void _move(double dy) {
    final live = _live;
    if (live == null) return;
    final by = -dy * widget.span * 2 / widget.height * _rate;
    final lo = widget.min, hi = widget.max;
    setState(() {
      if (_bar == 0) {
        for (var i = 0; i < 3; i++) {
          live[i] = (live[i] + by).clamp(lo, hi).toDouble();
        }
      } else {
        live[_bar - 1] = (live[_bar - 1] + by).clamp(lo, hi).toDouble();
      }
    });
    widget.onLive([...live]);
  }

  void _end() {
    final live = _live;
    if (live == null) return;
    setState(() => _live = null);
    widget.onCommit(live);
  }

  void _cancel() {
    if (_live == null) return;
    setState(() => _live = null);
    widget.onCancel();
  }

  /// A double-click: one bar back where it rests, the master moving all
  /// three back by the same amount. Already there writes nothing.
  void _reset(int bar) {
    final rgb = widget.rgb;
    final next = bar == 0
        ? [for (final c in rgb) c - wheelMaster(rgb) + widget.neutral]
        : [for (var i = 0; i < 3; i++) i == bar - 1 ? widget.neutral : rgb[i]];
    if ([for (var i = 0; i < 3; i++) (next[i] - rgb[i]).abs() < 1e-6]
        .every((x) => x)) {
      return;
    }
    widget.onStart();
    widget.onCommit(next);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final rgb = _rgb;
    final values = [wheelMaster(rgb), ...rgb];
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        for (var i = 0; i < 4; i++) ...[
          if (i > 0) SizedBox(width: i == 1 ? 10 : 6),
          GestureDetector(
            key: ValueKey<String>('grade-bar-$i'),
            behavior: HitTestBehavior.opaque,
            dragStartBehavior: DragStartBehavior.down,
            onDoubleTap: widget.enabled ? () => _reset(i) : null,
            onVerticalDragStart: widget.enabled ? (_) => _start(i) : null,
            onVerticalDragUpdate:
                widget.enabled ? (d) => _move(d.delta.dy) : null,
            onVerticalDragEnd: widget.enabled ? (_) => _end() : null,
            onVerticalDragCancel: widget.enabled ? _cancel : null,
            child: MouseRegion(
              cursor: widget.enabled
                  ? SystemMouseCursors.resizeUpDown
                  : MouseCursor.defer,
              child: CustomPaint(
                size: Size(12, widget.height),
                painter: _BarPainter(
                  at: ((values[i] - widget.neutral) / widget.span)
                      .clamp(-1.0, 1.0),
                  fill: !widget.enabled
                      ? t.hairline
                      : widget.keyed
                          ? t.animated
                          : widget.colours[i],
                  track: widget.enabled ? t.hairlineStrong : t.hairline,
                  mark: t.hairline,
                ),
              ),
            ),
          ),
        ],
      ],
    );
  }
}

/// One bar: a track, a mark where it rests, and a fill from there to the
/// value.
class _BarPainter extends CustomPainter {
  /// The value, -1 at the bottom to 1 at the top.
  final double at;
  final Color fill, track, mark;

  const _BarPainter(
      {required this.at,
      required this.fill,
      required this.track,
      required this.mark});

  @override
  void paint(Canvas canvas, Size size) {
    final x = size.width / 2;
    const top = 4.0;
    final bottom = size.height - 4;
    final mid = (top + bottom) / 2;
    canvas.drawLine(
      Offset(x, top),
      Offset(x, bottom),
      Paint()
        ..color = track
        ..strokeWidth = 1,
    );
    canvas.drawLine(
      Offset(x - 4, mid),
      Offset(x + 4, mid),
      Paint()
        ..color = mark
        ..strokeWidth = 1,
    );
    final y = mid - at * (bottom - top) / 2;
    canvas.drawRect(
      Rect.fromLTRB(x - 3, y < mid ? y : mid, x + 3, y < mid ? mid : y),
      Paint()..color = fill,
    );
    canvas.drawRRect(
      RRect.fromRectAndRadius(
          Rect.fromCenter(center: Offset(x, y), width: 10, height: 3),
          const Radius.circular(1.5)),
      Paint()..color = fill,
    );
  }

  @override
  bool shouldRepaint(_BarPainter old) =>
      old.at != at || old.fill != fill || old.track != track;
}
