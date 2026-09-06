// The Timeline's render-time column: what each layer's picture cost in the
// frame the playhead is on, and what each effect within it cost (docs/13 §7.1,
// docs/TODO.md "Layer and effect render-time indicator").
//
// **In plain terms.** "This comp is slow" is not something to guess about. With
// the column switched on, every layer row carries the milliseconds its own
// picture took in the last measured frame, and twirling a layer open puts the
// same number on each effect's heading — so the layer that is costing the
// session, and the effect inside it that is doing the costing, are both a
// glance away.
//
// **What it costs, and where the switch is.** Measuring makes the engine wait
// for the graphics card at every layer and every effect — honest, since a
// millisecond then means the work rather than the paperwork — and a measured
// frame is composited rather than served from a cache. It is on by default,
// because numbers are what the column is for; the clock in the bottom strip,
// beside the cache meters, turns it off for the session — and turning it off
// takes the column away entirely rather than leaving a row of dashes, so the
// Timeline gets its width back and the effect headings read as they did before
// this existed. Playback is never measured whatever the clock says.

import 'package:flutter/widgets.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../main.dart';
import '../state/render_timings.dart';
import '../widgets/controls.dart';

/// The column header: what the **whole frame** cost, which is the number the
/// rows below add up to — or the word "Time" while nothing is being measured.
///
/// **A readout, not a control.** It was a control once, and that is precisely
/// how the feature was reported broken: a header that says Time over a column
/// of dashes gives no hint that the header is a button, so the column simply
/// looked like it did not work. The switch now lives in the bottom strip beside
/// the cache meters ([RenderTimingsToggle]), where something that governs the
/// whole session belongs and where it can be seen without being looked for.
///
/// Three readings, three states: `Time` (not measuring), `…` (measuring, no
/// measured frame back yet), a number (measured — so a dash on a row below
/// genuinely means "not in that frame").
class TimingsHeaderCell extends StatelessWidget {
  const TimingsHeaderCell({super.key});

  /// The word for one stage, from the arb.
  static String stageWord(RenderStageKind kind) => switch (kind) {
        RenderStageKind.plan => l10n.stagePlan,
        RenderStageKind.decode => l10n.stageDecode,
        RenderStageKind.build => l10n.stageBuild,
        RenderStageKind.composite => l10n.stageComposite,
        RenderStageKind.present => l10n.stagePresent,
      };

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final timings =
        Provider.of<LumitUiState>(context, listen: false).renderTimings;
    return ListenableBuilder(
      listenable: timings,
      builder: (context, _) {
        final on = timings.measuring;
        final total = timings.totalMs;
        final stages = timings.stages;
        // The total, and — when one stage the rows can never itemise owns
        // most of it — that stage's name beside it: a 97 ms frame whose rows
        // are all cheap used to hang unexplained over the column, and the
        // usual culprit was the draw-list build, which belongs to no layer.
        final culprit = on ? dominantUnownedStage(stages) : null;
        final text = !on
            ? l10n.timeColumn
            : total == null
                ? '…'
                : culprit == null
                    ? formatRenderMs(total)
                    : '${formatRenderMs(total)} · ${stageWord(culprit)}';
        // The tooltip carries the full split while there is one, so where the
        // time went is a hover away even when no stage dominates.
        final split = stages.isEmpty
            ? l10n.tipRenderTime
            : stages
                .map((s) => '${stageWord(s.kind)} ${formatRenderMs(s.ms)}')
                .join(' · ');
        return LumitTooltip(
          message: split,
          child: Align(
            alignment: Alignment.centerRight,
            child: Padding(
              padding: const EdgeInsets.only(right: 4),
              child: Text(
                text,
                key: const ValueKey('tl-timings-header'),
                style: t.small,
                overflow: TextOverflow.ellipsis,
              ),
            ),
          ),
        );
      },
    );
  }
}

/// The session's measuring switch: a clock in the bottom strip, after the cache
/// meters. Lit in the accent while measuring.
///
/// **Why here and not on the column.** It governs the whole session and it
/// costs something to have on, which is the same shape of thing as the cache
/// meters it sits beside — and unlike a glyph inside a column header, it is
/// somewhere a person can find it without being told.
class RenderTimingsToggle extends StatelessWidget {
  const RenderTimingsToggle({super.key});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final timings =
        Provider.of<LumitUiState>(context, listen: false).renderTimings;
    return ListenableBuilder(
      listenable: timings,
      builder: (context, _) {
        final on = timings.measuring;
        return LumitTooltip(
          message: on ? l10n.tipStopMeasuring : l10n.tipMeasureRenderTimes,
          child: GestureDetector(
            key: const ValueKey('status-render-timings'),
            behavior: HitTestBehavior.opaque,
            onTap: () => timings.setMeasuring(!on),
            child: Padding(
              padding: const EdgeInsets.symmetric(horizontal: 2),
              child: lumitIcon(
                LumitIcon.stopwatch,
                size: iconSize,
                color: on ? t.accent : t.textMuted,
              ),
            ),
          ),
        );
      },
    );
  }
}

/// One measured cost, right-aligned so a column of them reads as numbers.
///
/// [layerId] and [effectId] are alternatives — a layer row gives the first, an
/// effect's heading the second.
///
/// Nothing at all while the clock in the bottom strip is off: the Timeline drops
/// the whole column then, and an effect's heading goes back to the shape it had
/// before this existed. A dash *while measuring* means the last measured frame
/// had no such row — it was hidden, outside its span, or inside a Precomp.
class TimingsCell extends StatelessWidget {
  final String? layerId;
  final String? effectId;

  const TimingsCell({super.key, this.layerId, this.effectId});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final timings =
        Provider.of<LumitUiState>(context, listen: false).renderTimings;
    return ListenableBuilder(
      listenable: timings,
      builder: (context, _) {
        final on = timings.measuring;
        final id = layerId ?? effectId;
        final ms = !on || id == null
            ? null
            : layerId != null
                ? timings.layerMs(id)
                : timings.effectMs(id);
        // Switched off, the column is not there at all — no cell, no dash, and
        // (in the Timeline) no column: an indicator nobody has asked for should
        // take no space and say nothing, which is also what makes the Effect
        // controls heading read as it did before this existed.
        if (!on) return const SizedBox.shrink();
        final cell = Align(
          alignment: Alignment.centerRight,
          child: Padding(
            padding: const EdgeInsets.only(right: 4),
            child: Text(
              ms == null ? '—' : formatRenderMs(ms),
              // Mono, because it is a number, and §7.1's mono-for-numbers rule
              // has no exceptions anywhere in the UI. **10**, the size every
              // other number in an outline row takes and the size the mockup
              // draws this one at; it was 9, a kicker's size on a value.
              style: t.mono.copyWith(fontSize: 10, color: t.textMuted),
              maxLines: 1,
              overflow: TextOverflow.clip,
            ),
          ),
        );
        return cell;
      },
    );
  }
}
