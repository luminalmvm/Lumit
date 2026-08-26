// The Timeline's toolbar and its column toggles: the readouts, the search, the
// mode tabs and the more menu across the top, and the kickers along the bottom
// that take a column group away.
//
// Split out of timeline_panel_frb.dart.

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:provider/provider.dart';
import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/splash.dart';
import '../state/comp_model.dart';
import '../state/settings.dart';
import '../state/timecode.dart';
import '../state/timeline_columns.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/time_readout.dart';
import 'timeline_extras_frb.dart';
import 'timeline_metrics_frb.dart';

/// **A control standing in one of the Timeline's two chrome rows**, grown to
/// the height the density states for them (K-512).
///
/// In plain terms: Regular's chrome rows are taller than they used to be, so
/// the tabs, the search well and the two readouts in them are told to stand
/// 20 tall instead of each measuring itself to something between 13 and 16 and
/// floating in a band of ground. Compact states no height at all — its rows
/// are the rows these controls were built for — and this hands the control
/// straight back, so nothing about Compact changed by a pixel.
///
/// A `HouseButton` handed a height centres its label in it; a well handed one
/// takes it in place of its own, because a tight constraint from above wins
/// over a box's own preferred size.
Widget timelineChromeControl(LumitTheme t, Widget child) {
  final height = t.density.timelineChromeControl;
  return height == null ? child : SizedBox(height: height, child: child);
}

/// The outline's toolbar (docs/07 §4.1, §12A.1): the timecode and frame
/// readouts at the far left, the layer search stretched across the middle as
/// an inset well, and the Layers / Graph mode segments at the far right — with
/// the master motion-blur and shy-filter buttons and the ⋯ menu (the
/// layer/work-area/marker commands the old full-width toolbar carried) between
/// the well and the segments.
///
/// **The Timeline's first chrome row**, and so `t.density.timelineChromeRow` —
/// **24** under Regular, 18 under Compact (K-512, docs/15 §12A.6). It used to
/// be a plain secondary row at 19 either way; the owner's ruling after desktop
/// testing is that this row is aimed at all day and 19 was too small to hit
/// comfortably. It plus the header row under it are still exactly what the
/// lane side spends on its ruler, which is what makes the two halves meet.
///
/// **This is where the hit floor gives way** (§7.2, K-452): the buttons in
/// this row are the row's own height, well under the 32 nothing interactive is
/// supposed to hit-test below. They are not given the floor by slop, because
/// there is nowhere to take it from — a few pixels above is the composition
/// tab strip and a few below is the column header's own drag-to-reorder and
/// resize seams, so an expanded target here would swallow a neighbour's
/// gesture rather than add one. Across, where a row like this is actually
/// aimed, they keep their room. What K-512 could do — and did — is give the
/// row itself more height and grow every control in it to match, which is
/// what [timelineChromeControl] is.
class Toolbar extends StatelessWidget {
  /// The read model, for the exact rate — no bridge calls in a build (K-184).
  final CompModel model;

  /// Listened to, not read: only the two readouts redraw as it moves.
  final ValueListenable<int> playhead;

  /// Where a typed time goes — the same take-hold-of-the-playhead move a drag
  /// on the ruler makes, so typing a time also stops the transport.
  final ValueChanged<int> onSeek;

  /// Which view is up, and how to ask for another (K-455).
  final TimelineMode mode;
  final ValueChanged<TimelineMode> onMode;
  final ValueChanged<String> onSearch;

  const Toolbar({super.key, 
    required this.model,
    required this.playhead,
    required this.onSeek,
    required this.mode,
    required this.onMode,
    required this.onSearch,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final (fpsNum, fpsDen) = model.fpsExact;
    final lastFrame = model.durationFrames - 1;
    return Container(
      height: t.density.timelineChromeRow,
      // The panel's own surface, ruled off from the column header below it —
      // the mockup draws both chrome rows on the panel ground with a
      // hairline under each, not as a raised strip.
      decoration: BoxDecoration(
        color: t.surface1,
        border: Border(bottom: BorderSide(color: t.hairline)),
      ),
      padding: const EdgeInsets.only(left: 10, right: 8),
      child: Row(
        children: [
          // The clock face and the frame count, both zero-based: frame 0 is
          // 00:00:00:00, so three seconds into a 24 fps comp reads f72.
          //
          // Both sit in slots wide enough for the longest thing they can say
          // and both can be typed into (K-287): a readout that resized itself
          // as it counted shoved the search field sideways through every
          // second of playback, and a time you can read is a time you should
          // be able to state. Anything outside the composition lands on its
          // nearest end.
          ValueListenableBuilder<int>(
            valueListenable: playhead,
            builder: (context, frame, _) => Row(
              children: [
                timelineChromeControl(
                    t,
                    TimeReadout(
                      key: const ValueKey('tl-timecode'),
                      frame: frame,
                      format: (f) => timecodeOfRate(f, fpsNum, fpsDen),
                      widthChars: timecodeChars(fpsNum, fpsDen),
                      // The clock is the row's first fact and reads at full
                      // strength; the frame count beside it is the same moment
                      // said again, so it stays muted (§12A.1).
                      style:
                          t.mono.copyWith(fontSize: 11, color: t.textPrimary),
                      parse: (text) => framesOfTimecode(text, fpsNum, fpsDen),
                      onCommit: onSeek,
                      minFrame: 0,
                      maxFrame: lastFrame,
                      tooltip: l10n.tipPlayheadTime,
                      // A **well**, because the clock can be typed into (K-460):
                      // the recess is what says so, and a time you can read is a
                      // time you should be able to state. It was bare text that
                      // happened to answer a click.
                      well: true,
                    )),
                const SizedBox(width: 4),
                timelineChromeControl(
                    t,
                    TimeReadout(
                      key: const ValueKey('tl-frame'),
                      frame: frame,
                      format: (f) => 'F$f',
                      // The `f`, the digits of the last frame, and one spare so a
                      // comp that grows past a power of ten does not start to
                      // twitch before the next rebuild.
                      widthChars: 2 + '${lastFrame < 0 ? 0 : lastFrame}'.length,
                      style: t.mono.copyWith(fontSize: 10, color: t.textMuted),
                      parse: _frameOfTyped,
                      onCommit: onSeek,
                      minFrame: 0,
                      maxFrame: lastFrame,
                      tooltip: l10n.tipFrameNumber,
                      well: true,
                      // Rests as `F48`, edits as `48` (K-460, capital by owner ruling). The `f` names the
                      // clock rather than counting in it, so the field holds the
                      // bare number and wears the letter again on commit — an
                      // edit that began by stepping over a letter began wrong.
                      editFormat: (f) => '$f',
                    )),
              ],
            ),
          ),
          // How many frames there are in all, after the frame the playhead is
          // on: `F48 /250`, as the owner reads it after desktop testing. The
          // mockup writes a space after the slash; on a real comp the phrase
          // then breaks into three marks — a number, a lone stroke, another
          // number — where it is one reading: *of* 250. The slash binds to the
          // count it introduces, and the space before it is what separates the
          // phrase from the frame counter. One muted colour throughout, so the
          // count matches the counter rather than fading a step further.
          // Outside the listener, because a comp's length does not move as the
          // playhead does; outside the well, because the comp's length is not
          // editable and a recess round it would say it was (K-460).
          const SizedBox(width: 4),
          Text(
            '/${model.durationFrames}',
            style: t.mono.copyWith(fontSize: 10, color: t.textMuted),
          ),
          // The search well, stretched between the frame counter and the mode
          // tabs (§12A.1) — with the outline's own [outlineGap] of air at each
          // side (owner, desktop testing). It had the chrome row's 2, which is
          // the gap between two chips of one run; a well is not a chip in a
          // run, and at 2 it read as being wedged between the counter and the
          // tabs. The 8 is the rhythm the compose boxes keep from one another
          // in the rows below, so the panel breathes the same everywhere.
          const SizedBox(width: outlineGap),
          Expanded(
              child: timelineChromeControl(
                  t, LayerSearchFrb(onChanged: onSearch, width: 1e9))),
          const SizedBox(width: outlineGap),
          // The two modes, at the far right of the row (§12A.1, K-529).
          // Kicker segments rather than icons: "Layers" and "Graph" are the
          // names of two shapes of the same panel, and a word says which one
          // is in force where two small glyphs made the reader guess.
          _modeTab(
            context,
            keyName: 'tl-view-lanes',
            label: l10n.timelineModeLayers,
            tip: l10n.tipLaneView,
            active: mode == TimelineMode.layers,
            onPressed: () => onMode(TimelineMode.layers),
          ),
          const SizedBox(width: 2),
          _modeTab(
            context,
            // Keeps the key the old Graph toolbar button had, so the graph
            // editor's own tests and muscle memory both still find it.
            keyName: 'tl-graph',
            label: l10n.timelineModeGraph,
            tip: l10n.tipGraphView,
            active: mode == TimelineMode.graph,
            onPressed: () => onMode(TimelineMode.graph),
          ),
        ],
      ),
    );
  }

  /// One of the three mode segments. The one in force wears the secondary
  /// button's outline and a `kickerOn` label; the others are frameless and
  /// muted. **No accent**: §3.1's accent list is closed, and a mode segment is
  /// not on it — which of the three is in force reads from the frame.
  Widget _modeTab(
    BuildContext context, {
    required String keyName,
    required String label,
    required String tip,
    required bool active,
    required VoidCallback onPressed,
  }) {
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: tip,
      // Grown to the chrome row's stated control height under Regular
      // (K-512): these three are the buttons the owner named as hard to hit.
      child: timelineChromeControl(
        t,
        HouseButton(
          key: ValueKey<String>(keyName),
          small: true,
          frameless: !active,
          padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
          onPressed: onPressed,
          child:
              Text(label.toUpperCase(), style: active ? t.kickerOn : t.kicker),
        ),
      ),
    );
  }

  /// A typed frame number, with or without the `f` the readout wears. Null for
  /// anything that is not a number at all, which leaves the readout alone.
  static int? _frameOfTyped(String text) {
    var trimmed = text.trim().toLowerCase();
    if (trimmed.startsWith('f') || trimmed.startsWith('F')) {
      trimmed = trimmed.substring(1);
    }
    return int.tryParse(trimmed.trim());
  }
}

/// One comp-wide switch in the bottom bar: an icon that lights in `accent`
/// while it is on.
Widget _compToggleButton(
  BuildContext context, {
  required String keyName,
  required LumitIcon icon,
  required bool on,
  required String tip,
  required VoidCallback onPressed,
}) {
  final t = ThemeScope.of(context).theme;
  return LumitTooltip(
    message: tip,
    child: HouseButton(
      key: ValueKey<String>(keyName),
      small: true,
      frameless: true,
      // No vertical padding: a 16px glyph plus the button's own 1px edge is
      // the whole of an 18px bottom bar (K-451), and 2px more spilled it.
      padding: const EdgeInsets.symmetric(horizontal: 4),
      onPressed: onPressed,
      child:
          lumitIcon(icon, size: iconSize, color: on ? t.accent : t.textMuted),
    ),
  );
}

/// The commands that used to line the full-width toolbar, one menu deep:
/// adding layers, the razor, the work area, markers and beat detection.
///
/// It rides in the panel's bottom bar with the other comp-wide switches
/// (§12A.1); it was in the timecode row until the redesign, and every command
/// and key in it is the one it always was.
Future<void> _showMoreMenu(
  BuildContext context, {
  required CompositionReference comp,
  required ValueListenable<int> playhead,
  required bool razor,
  required VoidCallback onToggleRazor,
  required VoidCallback onChanged,
}) async {
  final t = ThemeScope.of(context).theme;
  final box = context.findRenderObject();
  if (box is! RenderBox) return;
  final playheadNow = playhead.value;
  final picked = await showMenuAt<String>(
    context: context,
    // Anchored on the button. Opening from a *bottom* bar, the popup's own
    // layout pulls it back up on screen rather than running off below —
    // which is why the anchor no longer guesses at an offset.
    position: box.localToGlobal(Offset.zero),
    width: 190,
    rows: (close) => [
      MenuRow(
          key: const ValueKey('tl-add-layer'),
          onPressed: () => close('new-layer'),
          child: Text(l10n.newLayer)),
      MenuRow(
          key: const ValueKey('tl-razor'),
          onPressed: () => close('razor'),
          child: Text(razor ? l10n.disarmRazor : l10n.armRazor,
              style: razor ? t.body.copyWith(color: t.accent) : null)),
      MenuRow(
          key: const ValueKey('tl-work-in'),
          onPressed: () => close('work-in'),
          child: Text(l10n.workAreaStart)),
      MenuRow(
          key: const ValueKey('tl-work-out'),
          onPressed: () => close('work-out'),
          child: Text(l10n.workAreaEnd)),
      MenuRow(
          key: const ValueKey('tl-clear-work-area'),
          onPressed: () => close('work-clear'),
          child: Text(l10n.workAreaClear)),
      MenuRow(
          key: const ValueKey('tl-markers'),
          onPressed: () => close('markers'),
          child: Text(l10n.menuMarkers)),
      MenuRow(
          key: const ValueKey('tl-detect-beats'),
          onPressed: () => close('beats'),
          child: Text(l10n.menuDetectBeats)),
    ],
  );
  if (!context.mounted) return;
  switch (picked) {
    case 'new-layer':
      await _showLayerMenu(context, comp, onChanged);
    case 'razor':
      onToggleRazor();
    case 'work-in' || 'work-out':
      comp.setWorkArea(
        span: workAreaWith(
          comp: comp,
          current: comp.getWorkArea(),
          wanted: playheadNow,
          isStart: picked == 'work-in',
        ),
      );
      onChanged();
    case 'work-clear':
      comp.setWorkArea(span: null);
      onChanged();
    case 'markers':
      await showMarkerEditorFrb(
        context: context,
        comp: comp,
        playheadFrame: playheadNow,
      );
      onChanged();
    case 'beats':
      // Seconds-long on a long comp, so it runs off-thread and the markers
      // appear when it finishes; a comp with no audio, or a machine with no
      // pipeline, says so by doing nothing rather than by an alarm. The card
      // is up for those seconds so the silence is not mistaken for a command
      // that did not land, and it comes down either way.
      showBusyWhile(
        context.read<LumitState>().busy,
        l10n.detectingBeats,
        comp
            .detectBeats(sensitivityPercent: 50)
            .then<void>((_) => onChanged(), onError: (_) {}),
      );
    case _:
      return;
  }
}
Future<void> _showLayerMenu(
  BuildContext context,
  CompositionReference comp,
  VoidCallback onChanged,
) async {
  final box = context.findRenderObject();
  if (box is! RenderBox) return;
  final picked = await showMenuAt<VoidCallback>(
    context: context,
    position: box.localToGlobal(Offset(0, box.size.height + 2)),
    width: 190,
    rows: (close) => [
      // The row carries what it does, not a word to switch on: the label is
      // translated (K-303) and would no longer match an English case.
      for (final (label, add) in <(String, VoidCallback)>[
        (l10n.menuSolid, comp.addSolidLayer),
        (l10n.menuText, comp.addTextLayer),
        (l10n.menuCamera, comp.addCameraLayer),
        (l10n.menuPointLight, () => comp.addLightLayer(kind: 0)),
        (l10n.menuSpotLight, () => comp.addLightLayer(kind: 1)),
        (l10n.menuAreaLight, () => comp.addLightLayer(kind: 2)),
        (l10n.menuAdjustment, comp.addAdjustmentLayer),
        (l10n.menuNull, comp.addNullLayer),
        (l10n.menuSequence, comp.addSequenceLayer),
      ])
        MenuRow(onPressed: () => close(add), child: Text(label)),
    ],
  );
  if (picked == null) return;
  picked();
  onChanged();
}
/// The outline's end of the bottom bar (K-448, §12A.1): one kicker per
/// column group, lit while that group is drawn.
///
/// Kickers rather than buttons because these name *containers* (§7.1) — they
/// are the same words the column headers carry, and clicking one takes its
/// columns away so the outline pares down to names and bars. Nothing here
/// touches the document: it is what this panel shows, and it lives as long as
/// the session does.
class ColumnToggles extends StatelessWidget {
  final List<TimelineGroup> groups;
  final Set<TimelineGroup> hidden;
  final ValueChanged<TimelineGroup> onToggle;

  /// What the chrome says (K-440), read once by the panel and handed down.
  /// These three toggles are the setting's **first consumer**: in every mode
  /// but [ChromeLabels.words] they draw the set's own Switches, Modes and
  /// Parent glyphs, and the word arrives in the tooltip as it does everywhere.
  final ChromeLabels labels;

  /// The Animated filter, which belongs on this end of the bar because it is
  /// the same kind of statement as the toggles beside it: what the **outline**
  /// draws. Everything right of the strip's rule is the document's.
  final bool animatedOnly;
  final VoidCallback onToggleAnimated;

  const ColumnToggles({super.key, 
    required this.groups,
    required this.hidden,
    required this.onToggle,
    required this.labels,
    required this.animatedOnly,
    required this.onToggleAnimated,
    required this.comp,
    required this.model,
    required this.playhead,
    required this.razor,
    required this.onToggleRazor,
    required this.hideShy,
    required this.onToggleHideShy,
    required this.onChanged,
  });

  /// What the comp-wide switches act on, and the read model they read their
  /// state from (K-184: no bridge call in a build).
  final CompositionReference comp;
  final CompModel model;

  /// Read when a command in the ⋯ menu is picked, never per rebuild.
  final ValueListenable<int> playhead;
  final bool razor;
  final VoidCallback onToggleRazor;
  final bool hideShy;
  final VoidCallback onToggleHideShy;
  final VoidCallback onChanged;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final mbOn = model.motionBlurEnabled;
    return Container(
      height: t.density.secondaryRow,
      color: t.surface2,
      padding: const EdgeInsets.symmetric(horizontal: 10),
      // Scrolls sideways when the outline is narrow — the same answer the
      // toolbar and the lane bar give; an overflow stripe is a layout fault.
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: Row(
          children: [
            for (final group in groups) ...[
              LumitTooltip(
                message: l10n.tipToggleColumns(columnGroupLabel(group)),
                child: HouseButton(
                  key: ValueKey<String>('tl-column-${group.name}'),
                  small: true,
                  frameless: true,
                  padding: const EdgeInsets.symmetric(horizontal: 4),
                  onPressed: () => onToggle(group),
                  child: labels == ChromeLabels.words
                      ? Text(
                          columnGroupLabel(group).toUpperCase(),
                          style: hidden.contains(group) ? t.kicker : t.kickerOn,
                        )
                      // The glyph takes the word's own two strengths: muted
                      // while the column is hidden, foreground while it is
                      // drawn — the same reading, in the same two colours.
                      : glyph.LumitIcon(
                          _columnGlyph(group),
                          size: iconSize,
                          colour: hidden.contains(group)
                              ? t.textMuted
                              : t.textPrimary,
                          semanticLabel: columnGroupLabel(group),
                        ),
                ),
              ),
              const SizedBox(width: 4),
            ],
            // The Animated filter, last of the outline's own marks (K-441,
            // 6.43). One toggle rather than the withdrawn Keys sheet's *Show —
            // All / Animated* pair: two states are two states, and a word that
            // reads like its neighbours on a strip of them says which is in
            // force without spending a row on saying so twice.
            LumitTooltip(
              message:
                  animatedOnly ? l10n.tipAnimatedShowing : l10n.tipAnimatedOnly,
              child: HouseButton(
                key: const ValueKey('tl-filter-animated'),
                small: true,
                frameless: true,
                padding: const EdgeInsets.symmetric(horizontal: 4),
                onPressed: onToggleAnimated,
                child: labels == ChromeLabels.words
                    ? Text(l10n.filterAnimated.toUpperCase(),
                        style: animatedOnly ? t.kickerOn : t.kicker)
                    : glyph.LumitIcon(
                        LumitIcons.animated,
                        size: iconSize,
                        colour: animatedOnly ? t.textPrimary : t.textMuted,
                        semanticLabel: l10n.filterAnimated,
                      ),
              ),
            ),
            const SizedBox(width: 4),
            // **The two groups read apart** (§12A.1): everything left of this
            // rule says which *columns* the outline draws; everything right of
            // it is comp-wide — shy, master motion blur, and the overflow of
            // commands. They shared the timecode row until the redesign, where
            // a column toggle and a document switch sat shoulder to shoulder
            // with nothing to say they were different kinds of thing.
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 4),
              child: Container(width: 1, height: 10, color: t.hairlineStrong),
            ),
            _compToggleButton(
              context,
              keyName: 'tl-hide-shy',
              icon: LumitIcon.shy,
              on: hideShy,
              tip: hideShy ? l10n.tipShyHidden : l10n.tipHideShy,
              onPressed: onToggleHideShy,
            ),
            _compToggleButton(
              context,
              keyName: 'tl-mb-master',
              icon: LumitIcon.motionBlur,
              on: mbOn,
              tip: mbOn
                  ? l10n.tipMasterMotionBlurOn
                  : l10n.tipMasterMotionBlurOff,
              onPressed: () {
                comp.setMotionBlurEnabled(on_: !mbOn);
                onChanged();
              },
            ),
            Builder(
              builder: (menuContext) => HouseButton(
                key: const ValueKey('tl-more'),
                small: true,
                frameless: true,
                padding: const EdgeInsets.symmetric(horizontal: 5),
                onPressed: () => _showMoreMenu(
                  menuContext,
                  comp: comp,
                  playhead: playhead,
                  razor: razor,
                  onToggleRazor: onToggleRazor,
                  onChanged: onChanged,
                ),
                child: Text('⋯', style: t.small),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// The set's own glyph for a column group (K-440) — the one the drawing gives
/// that word. Only the three toggleable groups can be asked: the rest are
/// never drawn as a toggle.
String _columnGlyph(TimelineGroup group) => switch (group) {
      TimelineGroup.switches => LumitIcons.switches,
      TimelineGroup.render => LumitIcons.modes,
      TimelineGroup.parent => LumitIcons.parent,
      // Not toggleable, and so never asked; answered rather than thrown for,
      // because a glyph is not worth a crash.
      _ => LumitIcons.label,
    };
