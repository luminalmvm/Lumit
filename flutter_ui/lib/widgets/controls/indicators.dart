// The two readouts that only report: the progress bar and the gesture pill.

import 'package:flutter/material.dart';

import 'base.dart';

/// The house progress bar: a fraction of accent fill on a `surface3` track.
/// One shape for the status line's export and cache meters and the update
/// download, which had each hand-rolled their own.
class HouseProgressBar extends StatelessWidget {
  final double fraction;
  final double height;
  const HouseProgressBar({super.key, required this.fraction, this.height = 4});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final radius = BorderRadius.circular(height / 2);
    return Container(
      height: height,
      decoration: BoxDecoration(color: t.surface3, borderRadius: radius),
      child: FractionallySizedBox(
        alignment: Alignment.centerLeft,
        widthFactor: fraction.clamp(0.0, 1.0),
        child: Container(
          decoration: BoxDecoration(color: t.accent, borderRadius: radius),
        ),
      ),
    );
  }
}

/// The live readout a gesture carries with it: a small `surface4` pill of 8px
/// mono, drawn beside the thing being moved and gone the moment it is let go
/// (docs/impl/timeline-interaction.md P1, §4.2/§6.2).
///
/// In plain terms: while you drag a keyframe, a tiny label rides next to the
/// pointer saying what frame and value it has reached, so you do not have to
/// look away at a readout somewhere else. It never appears at rest.
///
/// The same shape as the key block's badge — one pill, one size, wherever the
/// Timeline says a number under the hand.
class HintPill extends StatelessWidget {
  final String text;
  const HintPill({super.key, required this.text});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return IgnorePointer(
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
        decoration: BoxDecoration(
          color: t.surface4,
          borderRadius: BorderRadius.circular(2),
        ),
        child: Text(
          text,
          style: t.mono.copyWith(fontSize: 8, color: t.textPrimary),
        ),
      ),
    );
  }
}
