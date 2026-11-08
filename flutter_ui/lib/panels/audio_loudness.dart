// The master's loudness as three quiet numbers, under the Audio panel's
// levels and beside the Mixer's Master meter: short-term, integrated and the
// true peak. A click on the integrated number starts it afresh.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'audio_meters_feed.dart';

/// How the numbers stand: side by side, stacked with each mark over its number
/// for a strip, or the integrated number alone where a strip has no room.
enum LoudnessLayout { row, column, integrated }

class LoudnessReadout extends StatelessWidget {
  final AudioMeterFeed feed;

  final LoudnessLayout layout;

  const LoudnessReadout(
      {super.key, required this.feed, this.layout = LoudnessLayout.row});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final mark = t.mono.copyWith(fontSize: 9, color: t.textMuted);
    return ValueListenableBuilder<LoudnessLevels>(
      valueListenable: feed.loudness,
      builder: (context, l, _) {
        Widget cell(String key, String letters, String name, double value,
                {VoidCallback? onTap}) =>
            LumitTooltip(
              message: name,
              child: HoverMark(
                cursor: onTap == null
                    ? MouseCursor.defer
                    : SystemMouseCursors.click,
                builder: (context, lit) {
                  final number = Text(
                    value.isFinite
                        ? value.toStringAsFixed(1)
                        : l10n.volumeNegInf,
                    key: ValueKey<String>(key),
                    maxLines: 1,
                    softWrap: false,
                    // Nothing measured recedes, like the label beside it.
                    style: mark.copyWith(
                      color: Color.lerp(
                          value.isFinite ? t.textSecondary : t.textMuted,
                          t.textPrimary,
                          onTap == null ? 0 : lit),
                    ),
                  );
                  final words = Text(letters,
                      maxLines: 1, softWrap: false, style: mark);
                  final body = layout == LoudnessLayout.column
                      ? Column(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          mainAxisSize: MainAxisSize.min,
                          children: [words, number],
                        )
                      : Row(
                          mainAxisSize: MainAxisSize.min,
                          children: [words, const SizedBox(width: 4), number],
                        );
                  return GestureDetector(
                    behavior: HitTestBehavior.opaque,
                    onTap: onTap,
                    child: body,
                  );
                },
              ),
            );
        final integrated = cell('loudness-integrated',
            l10n.loudnessIntegratedMark, l10n.loudnessIntegrated, l.integrated,
            onTap: resetLoudness);
        if (layout == LoudnessLayout.integrated) return integrated;
        final cells = [
          cell('loudness-short', l10n.loudnessShortTermMark,
              l10n.loudnessShortTerm, l.shortTerm),
          integrated,
          cell('loudness-peak', l10n.loudnessTruePeakMark,
              l10n.loudnessTruePeak, l.truePeak),
        ];
        return layout == LoudnessLayout.column
            ? Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                mainAxisSize: MainAxisSize.min,
                children: [
                  for (var i = 0; i < cells.length; i++) ...[
                    if (i > 0) const SizedBox(height: 6),
                    cells[i],
                  ],
                ],
              )
            : Row(
                children: [
                  for (var i = 0; i < cells.length; i++) ...[
                    if (i > 0) const SizedBox(width: 14),
                    cells[i],
                  ],
                ],
              );
      },
    );
  }
}
