// The house slider: a thin track with a handle and an optional reading.

import 'package:flutter/material.dart';

import 'base.dart';

/// A thin themed slider. `commitOnRelease` reproduces the UI-scale rule
/// (K-117): the dragged value shows live but `onChanged` fires on release.
class HouseSlider extends StatefulWidget {
  final double value;
  final double min;
  final double max;
  final double? step;
  final int decimals;
  final String? suffix;
  final bool commitOnRelease;

  /// How wide the track is drawn. The default suits a settings row; a control
  /// in a toolbar wants less.
  final double width;

  /// Whether the number is drawn beside the track.
  ///
  /// Off for a slider whose value is already said elsewhere — the Timeline's
  /// zoom says it in a tooltip, and a readout repeating it would cost the
  /// bottom bar room it does not have.
  final bool showValue;
  final ValueChanged<double> onChanged;

  /// Called instead of [onChanged] while the handle is being **dragged**, for
  /// a control whose live value costs something the committed one does not —
  /// the Timeline's zoom applies a drag at once and only flies for a tap
  /// (K-293). Unset, a drag reports through [onChanged] as it always did.
  final ValueChanged<double>? onChangeLive;

  /// Fired once when a drag begins, before the first [onChangeLive] — for a
  /// caller that fixes something at the start of the gesture and holds it to
  /// the end (the Timeline's zoom anchors on the playhead *once* per drag,
  /// K-319). Omitted by callers with nothing to fix.
  final VoidCallback? onChangeStart;

  /// Fired once when a drag ends, after the last tick.
  final VoidCallback? onChangeEnd;

  const HouseSlider({
    super.key,
    required this.value,
    required this.min,
    required this.max,
    required this.onChanged,
    this.step,
    this.decimals = 2,
    this.suffix,
    this.commitOnRelease = false,
    this.width = 140,
    this.showValue = true,
    this.onChangeLive,
    this.onChangeStart,
    this.onChangeEnd,
  });

  @override
  State<HouseSlider> createState() => _HouseSliderState();
}

class _HouseSliderState extends State<HouseSlider> {
  double? _pending;

  double get _shown => _pending ?? widget.value;

  double _fromDx(double dx, double width) {
    var v =
        widget.min + (dx / width).clamp(0.0, 1.0) * (widget.max - widget.min);
    final s = widget.step;
    if (s != null && s > 0) v = (v / s).round() * s;
    return v.clamp(widget.min, widget.max).toDouble();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final width = widget.width;
    final frac =
        ((_shown - widget.min) / (widget.max - widget.min)).clamp(0.0, 1.0);
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTapUp: (d) => widget.onChanged(_fromDx(d.localPosition.dx, width)),
          onHorizontalDragStart: (_) => widget.onChangeStart?.call(),
          onHorizontalDragUpdate: (d) {
            final v = _fromDx(d.localPosition.dx, width);
            if (widget.commitOnRelease) {
              setState(() => _pending = v);
              // Held back from the *document*, not from the picture: a caller
              // with a live channel (an effect parameter's preview render)
              // still sees every tick, and only the release commits. Without
              // this the two options were exclusive, and a slider could either
              // preview or commit once, never both.
              widget.onChangeLive?.call(v);
            } else {
              (widget.onChangeLive ?? widget.onChanged)(v);
            }
          },
          onHorizontalDragEnd: (_) {
            if (_pending != null) {
              widget.onChanged(_pending!);
              setState(() => _pending = null);
            }
            widget.onChangeEnd?.call();
          },
          onHorizontalDragCancel: () => widget.onChangeEnd?.call(),
          child: SizedBox(
            width: width,
            height: 16,
            child: CustomPaint(
              painter: _SliderPainter(
                // The mockups' own track and knob: a `hairline_strong` rule
                // with a `text_secondary` handle on it. The track had been a
                // `surface0` recess, which spends a fourth grey on a groove
                // two pixels tall (§2.1), and the knob a `text_primary` dot,
                // which read brighter than the value it points at.
                track: t.hairlineStrong,
                fill: t.accent,
                knob: t.textSecondary,
                frac: frac,
              ),
            ),
          ),
        ),
        if (widget.showValue) ...[
          const SizedBox(width: 8),
          Text(
            '${_shown.toStringAsFixed(widget.decimals)}${widget.suffix ?? ''}',
            style: t.bodyPrimary,
          ),
        ],
      ],
    );
  }
}

class _SliderPainter extends CustomPainter {
  final Color track, fill, knob;
  final double frac;
  const _SliderPainter({
    required this.track,
    required this.fill,
    required this.knob,
    required this.frac,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final y = size.height / 2;
    final trackRect = RRect.fromRectAndRadius(
      Rect.fromLTWH(0, y - 2, size.width, 4),
      const Radius.circular(2),
    );
    canvas.drawRRect(trackRect, Paint()..color = track);
    canvas.drawRRect(
      RRect.fromRectAndRadius(
        Rect.fromLTWH(0, y - 2, size.width * frac, 4),
        const Radius.circular(2),
      ),
      Paint()..color = fill,
    );
    canvas.drawCircle(Offset(size.width * frac, y), 5, Paint()..color = knob);
  }

  @override
  bool shouldRepaint(_SliderPainter old) =>
      old.frac != frac || old.fill != fill || old.track != track;
}
