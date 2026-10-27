// The two readouts that only report: the progress bar and the gesture pill.

import 'package:flutter/material.dart';

import 'base.dart';
import 'motion.dart';

/// The house progress bar: a fraction of accent fill on a `surface3` track.
/// One shape for the status line's export and cache meters and the update
/// download, which had each hand-rolled their own.
class HouseProgressBar extends StatelessWidget {
  final double fraction;
  final double height;

  /// Whether the fill runs to each new reading instead of jumping to it. Off
  /// for a bar that something else is already moving every frame, the
  /// indeterminate sweep, which would only be smeared by a second easing.
  final bool eased;

  /// What the fill is drawn in when it is not the accent: the sweep, for a
  /// job that is Lumit Pro's.
  final Gradient? fill;

  const HouseProgressBar({
    super.key,
    required this.fraction,
    this.height = 4,
    this.eased = true,
    this.fill,
  });

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final radius = BorderRadius.circular(height / 2);
    return Container(
      height: height,
      decoration: BoxDecoration(color: t.surface3, borderRadius: radius),
      // A job reports its progress in steps, and a bar that jumped from one
      // to the next read as a stutter in the job. The fill runs to each new
      // reading instead (docs/15 §8). It is paint inside a fixed track, so
      // nothing around the bar is laid out again.
      child: TweenAnimationBuilder<double>(
        tween: Tween<double>(end: fraction.clamp(0.0, 1.0)),
        duration: eased ? scope.motion.mark.duration : Duration.zero,
        curve: Curves.easeOut,
        builder: (context, shown, child) => FractionallySizedBox(
          alignment: Alignment.centerLeft,
          widthFactor: shown.clamp(0.0, 1.0),
          child: child,
        ),
        child: Container(
          decoration: BoxDecoration(
              color: fill == null ? t.accent : null,
              gradient: fill,
              borderRadius: radius),
        ),
      ),
    );
  }
}

/// The live readout a gesture carries with it: a small `surface4` pill of 9px
/// mono, the floor every word is set at, drawn beside the thing being moved and gone the moment it is let go
/// (docs/impl/timeline-interaction.md P1, §4.2/§6.2).
///
/// In plain terms: while you drag a keyframe, a tiny label rides next to the
/// pointer saying what frame and value it has reached, so you do not have to
/// look away at a readout somewhere else. It never appears at rest.
///
/// The same shape as the key block's badge — one pill, one size, wherever the
/// Timeline says a number under the hand.
///
/// A pill answers the pointer only when given [onTap]: the graph's selected-key
/// readout opens that key's ease fields from it. Every other pill rides under
/// a gesture and lets the pointer through, as it always has.
class HintPill extends StatelessWidget {
  final String text;

  /// Called with the tap's window position, so what opens can sit beside it.
  final void Function(Offset globalPosition)? onTap;

  const HintPill({super.key, required this.text, this.onTap});

  @override
  Widget build(BuildContext context) {
    final tap = onTap;
    if (tap == null) return IgnorePointer(child: face(context, text));
    // One that opens something says so under the pointer.
    return HoverMark(
      swells: true,
      builder: (context, lit) => GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTapUp: (d) => tap(d.globalPosition),
        child: face(context, text, lit: lit),
      ),
    );
  }

  /// The pill itself, [lit] by as much as the pointer is on one that can be
  /// pressed: a brighter fill inside an edge. [radius] is the shape's action
  /// corner unless a drawing states its own.
  static Widget face(BuildContext context, String text,
      {double lit = 0, double? radius}) {
    final t = ThemeScope.of(context).theme;
    return Container(
      // A pixel less than the drawing's 4 and 1, which the edge takes: it is
      // always there and clear at rest, so lighting the pill moves nothing.
      padding: const EdgeInsets.symmetric(horizontal: 3),
      decoration: BoxDecoration(
        color: Color.lerp(t.surface4, t.hairlineStrong, lit),
        borderRadius: BorderRadius.circular(radius ?? t.tokens.actionRadius),
        border: Border.all(color: t.textMuted.withValues(alpha: lit)),
      ),
      child: Text(
        text,
        style:
            t.mono.copyWith(fontSize: t.caption.fontSize, color: t.textPrimary),
      ),
    );
  }
}
