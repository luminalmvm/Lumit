// The Timeline's toolbar and its column toggles: the readouts, the search, the
// mode tabs and the more menu across the top, and the kickers along the bottom
// that take a column group away.
//
// Split out of timeline_panel_frb.dart.

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/beats.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:provider/provider.dart';
import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/splash.dart';
import '../state/beats_notice.dart';
import '../state/comp_model.dart';
import '../state/settings.dart';
import '../state/timecode.dart';
import '../state/timeline_columns.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/time_readout.dart';
import 'graph_channels.dart';
import 'graph_maths.dart';
import 'timeline_extras_frb.dart';
import 'timeline_metrics_frb.dart';
import 'timeline_navigator.dart';

/// **A control standing in one of the Timeline's two chrome rows**, grown to
/// the height the density states for them.
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
/// **24** under Regular, 18 under Compact (docs/15 §12A.6) — **plus the
/// navigator's band** (the owner's ruling): the time navigator
/// stands over the lane area alone, and this row grows by exactly its band to
/// meet the panel top, where the strip's blank half used to leave a sliver of
/// dead ground above it. The row used to be a plain secondary row at 19 either
/// way; the owner's ruling after desktop testing is that this row is aimed at
/// all day and 19 was too small to hit comfortably. It plus the header row
/// under it face exactly what the lane side spends on the strip and its ruler,
/// which is what makes the two halves meet.
///
/// **This is where the hit floor gives way** (§7.2): the buttons in
/// this row are the row's own height, well under the 32 nothing interactive is
/// supposed to hit-test below. They are not given the floor by slop, because
/// there is nowhere to take it from — a few pixels above is the composition
/// tab strip and a few below is the column header's own drag-to-reorder and
/// resize seams, so an expanded target here would swallow a neighbour's
/// gesture rather than add one. Across, where a row like this is actually
/// aimed, they keep their room. What an earlier change could do — and did — is
/// give the row itself more height and grow every control in it to match,
/// which is what [timelineChromeControl] is.
class Toolbar extends StatelessWidget {
  /// The read model, for the exact rate — no bridge calls in a build.
  final CompModel model;

  /// Listened to, not read: only the two readouts redraw as it moves.
  final ValueListenable<int> playhead;

  /// Where a typed time goes — the same take-hold-of-the-playhead move a drag
  /// on the ruler makes, so typing a time also stops the transport.
  final ValueChanged<int> onSeek;

  /// Which view is up, and how to ask for another.
  final TimelineMode mode;
  final ValueChanged<TimelineMode> onMode;
  final ValueChanged<String> onSearch;

  const Toolbar({
    super.key,
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
      key: const ValueKey('tl-toolbar'),
      height: t.density.timelineChromeRow + TimelineNavigator.band,
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
          // and both can be typed into: a readout that resized itself
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
                      // A **well**, because the clock can be typed into:
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
                      // Rests as `F48`, edits as `48` (capital by owner ruling). The `f` names the
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
          // editable and a recess round it would say it was.
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
          // The two modes, at the far right of the row (§12A.1).
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
      // Grown to the chrome row's stated control height under Regular:
      // these three are the buttons the owner named as hard to hit.
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
      // the whole of an 18px bottom bar, and 2px more spilled it.
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
      // appear when it finishes. The card is up for those seconds so the
      // silence is not mistaken for a command that did not land, and it comes
      // down either way — and a comp with nothing sounding in it now says so
      // on the status line rather than by leaving the Timeline unchanged.
      final app = context.read<LumitState>();
      showBusyWhile(
        app.busy,
        l10n.detectingBeats,
        comp.detectBeats(options: BridgeBeatOptions.standard()).then<void>(
          (found) {
            onChanged();
            app.postNotice(found.placed == 0
                ? l10n.beatsNoneFound
                : beatsFoundNotice(found));
          },
          onError: (_) => app.postNotice(l10n.beatsNoSound),
        ),
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
      // translated and would no longer match an English case.
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

/// **The key commands, at the outline's foot** — the interpolation words and
/// what goes with them, standing under the list they act on.
///
/// They stood on the lane bar until the owner moved them here: that bar had
/// grown into the longest strip of buttons in the panel while this end of the
/// same row had room to spare, and the commands act on a *key selection*,
/// which is made in the outline above them. The bar under the lanes is left
/// with the two things that are about the lanes — the zoom and the scrollbar.
///
/// The word *Interpolation* went with the move. It labelled the four words
/// beside it, and Linear / Hold / Ease / Bezier need no telling what they are;
/// a kicker naming a run of kickers only spent room the outline's foot has
/// less of than the lane bar had.
///
/// Two shapes, one per view. In **Layers** it is the keyframe strip:
/// the four interpolations — and nothing after them, since the owner removed
/// Reverse, Copy and Paste at playhead from the bar (2026-08-31).
/// In **graph view** it is the graph's own commands (docs/07 §5.3): the eases,
/// the tangent modes, the value/speed lens and the auto-fit toggle. Every
/// button keeps the face, the tooltip and the widget key it had on the lane
/// bar — this is a move, not a redesign.
class KeyCommandStrip extends StatelessWidget {
  /// Set in **Layers mode**: the keyframe strip.
  final bool strip;

  /// Set in graph view; null leaves the graph's own commands out.
  final GraphLens? lens;
  final ValueChanged<GraphLens>? onLens;
  final bool autoFit;
  final VoidCallback? onToggleAutoFit;
  final ValueChanged<BridgeSideInterp>? onInterp;

  /// A tangent mode chosen for the selected keys — Auto / Clamp / Free (§6.3).
  final ValueChanged<TangentMode>? onTangentMode;

  /// The Easing… button pressed, with the button's own context so a popup can
  /// be anchored to it. Whether that is a popup or a docked panel is the
  /// panel's decision, not this strip's.
  final ValueChanged<BuildContext>? onOpenEasing;

  /// The Ease word pressed, with its own context so the popover can be
  /// anchored to it — the same box the block badge opens.
  final ValueChanged<BuildContext>? onEaseBlock;

  const KeyCommandStrip({
    super.key,
    this.strip = false,
    this.lens,
    this.onLens,
    this.autoFit = true,
    this.onToggleAutoFit,
    this.onInterp,
    this.onTangentMode,
    this.onOpenEasing,
    this.onEaseBlock,
  });

  Widget _button(
    LumitTheme t, {
    required String keyName,
    required String label,
    required String tip,
    required bool on,
    required VoidCallback onPressed,
  }) =>
      LumitTooltip(
        message: tip,
        child: HouseButton(
          key: ValueKey<String>(keyName),
          small: true,
          // The one in force wears the button's own `hairline_strong` idle
          // edge; the rest are frameless. **No accent** — §3.1's accent list
          // is closed and a lens or an ease is not on it, so which is in force
          // reads from the frame and the brighter label, as the mode tabs do.
          frameless: !on,
          // 18px bottom bar: one pixel of the button's own edge is
          // all the room a 9px kicker leaves above and below it.
          padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
          onPressed: onPressed,
          child: Text(label.toUpperCase(), style: on ? t.kickerOn : t.kicker),
        ),
      );

  /// A strip command drawn as a glyph of the set: the interpolation
  /// entries, whose shapes — a line, a step, a curve, a handled curve — say
  /// more at 16px than four capitalised words did. The word is the tooltip,
  /// which is the control's name, and the semantic label for a reader.
  Widget _glyphButton(
    LumitTheme t, {
    required String keyName,
    required String mark,
    required String word,
    required VoidCallback onPressed,
  }) =>
      LumitTooltip(
        message: word,
        child: HouseButton(
          key: ValueKey<String>(keyName),
          small: true,
          frameless: true,
          // The comp-toggle buttons' own padding: a 16px glyph plus the
          // button's 1px edge is the whole of an 18px bottom bar.
          padding: const EdgeInsets.symmetric(horizontal: 4),
          onPressed: onPressed,
          // A command, not a toggle: it rests at the column's muted strength,
          // the way the magnet and the shy filter rest when off.
          child: glyph.LumitIcon(mark,
              size: iconSize, colour: t.textMuted, semanticLabel: word),
        ),
      );

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      key: const ValueKey('tl-key-commands'),
      height: t.density.secondaryRow,
      // The same ground as the column toggles it stands beside: the two are
      // one bar, split between what the outline shows and what the selection
      // can be told to do.
      color: t.surface2,
      padding: const EdgeInsets.only(left: 10),
      // Scrolls sideways when the outline is narrow — the same answer the
      // toolbar and the lane bar give; an overflow stripe is a layout fault.
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: Row(
          children: [
            // Two gaps run through this strip: 2 between the chips of one
            // segmented run, 12 between one run and the next, so the runs read
            // as groups rather than as one long strip of buttons.
            if (strip) ...[
              // Glyphs, not words (the owner's ask): the set's own
              // marks for the four interpolations, each named by its tooltip.
              _glyphButton(t,
                  keyName: 'keys-interp-linear',
                  mark: LumitIcons.linear,
                  word: l10n.easeLinear,
                  onPressed: () =>
                      onInterp?.call(const BridgeSideInterp.linear())),
              const SizedBox(width: 2),
              _glyphButton(t,
                  keyName: 'keys-interp-hold',
                  mark: LumitIcons.hold,
                  word: l10n.easeHold,
                  onPressed: () =>
                      onInterp?.call(const BridgeSideInterp.hold())),
              const SizedBox(width: 2),
              // The shaped ease, one step along from the flat three: the same
              // selection, a curve chosen by name instead of a constant. Its
              // own Builder so the popover can be anchored to the glyph that
              // opened it.
              Builder(
                builder: (buttonContext) => _glyphButton(t,
                    keyName: 'keys-interp-ease',
                    mark: LumitIcons.easeInOut,
                    word: l10n.keysEase,
                    onPressed: () => onEaseBlock?.call(buttonContext)),
              ),
              const SizedBox(width: 2),
              _glyphButton(t,
                  keyName: 'keys-interp-bezier',
                  mark: LumitIcons.bezier,
                  word: l10n.easeBezier,
                  onPressed: () => onInterp?.call(easyEase)),
              const SizedBox(width: 12),
              // No second run: Reverse, Copy and Paste at playhead left the
              // strip on the owner's ruling (2026-08-31, desktop testing).
              // Copy and Paste keep their Ctrl+C / Ctrl+V roads; the four
              // interpolations are the whole of the bar.
            ],
            if (lens != null) ...[
              // The selected keys' easing, one click each — the F9 family's
              // buttons (docs/07 §5.3).
              _button(t,
                  keyName: 'graph-interp-linear',
                  label: l10n.easeLinear,
                  tip: l10n.tipLinearKeyframes,
                  on: false,
                  onPressed: () =>
                      onInterp?.call(const BridgeSideInterp.linear())),
              const SizedBox(width: 2),
              _button(t,
                  keyName: 'graph-interp-bezier',
                  label: l10n.easeBezier,
                  tip: l10n.tipEasyEase,
                  on: false,
                  onPressed: () => onInterp?.call(easyEase)),
              const SizedBox(width: 2),
              _button(t,
                  keyName: 'graph-interp-hold',
                  label: l10n.easeHold,
                  tip: l10n.tipHoldKeyframes,
                  on: false,
                  onPressed: () =>
                      onInterp?.call(const BridgeSideInterp.hold())),
              // The shaped ease, one step along from the one-click three: same
              // selection, a curve instead of a constant. Its own Builder so
              // the popup can find where this button is; the popup layout
              // slides it up into view.
              //
              // Value lens only. The box draws a shape against the value's own
              // travel, so a curve stamped while the speed lens is up would
              // land on the value graph — a change the user cannot see in the
              // view they drew it in. The one-click three above stay in both
              // lenses: a side's interp means the same thing either way.
              if (lens == GraphLens.value) ...[
                const SizedBox(width: 2),
                Builder(
                  builder: (buttonContext) => _button(t,
                      keyName: 'graph-interp-easing',
                      label: l10n.easeCustom,
                      tip: l10n.tipEasingEditor,
                      on: false,
                      onPressed: () => onOpenEasing?.call(buttonContext)),
                ),
              ],
              const SizedBox(width: 12),
              // Tangents — Auto / Clamp / Free (§6.3), between the ease
              // presets and the lens pair. A run of three like the eases
              // beside them, and unlit for the same reason: these are things
              // to *do* to the selection, and a selection spanning two modes
              // has no one answer to light. Which mode a side is in is legible
              // where it matters — in the handle, which stops following its
              // neighbours the moment it is dragged.
              _button(t,
                  keyName: 'graph-tangent-auto',
                  label: l10n.graphTangentAuto,
                  tip: l10n.tipTangentAuto,
                  on: false,
                  onPressed: () => onTangentMode?.call(TangentMode.auto)),
              const SizedBox(width: 2),
              _button(t,
                  keyName: 'graph-tangent-clamp',
                  label: l10n.graphTangentClamp,
                  tip: l10n.tipTangentClamp,
                  on: false,
                  onPressed: () => onTangentMode?.call(TangentMode.clamp)),
              const SizedBox(width: 2),
              _button(t,
                  keyName: 'graph-tangent-free',
                  label: l10n.graphTangentFree,
                  tip: l10n.tipTangentFree,
                  on: false,
                  onPressed: () => onTangentMode?.call(TangentMode.free)),
              const SizedBox(width: 12),
              _button(t,
                  keyName: 'graph-lens-value',
                  label: l10n.clipboardValueColumn,
                  tip: l10n.tipValueGraph,
                  on: lens == GraphLens.value,
                  onPressed: () => onLens?.call(GraphLens.value)),
              const SizedBox(width: 2),
              _button(t,
                  keyName: 'graph-lens-speed',
                  label: l10n.graphSpeed,
                  tip: l10n.tipSpeedGraph,
                  on: lens == GraphLens.speed,
                  onPressed: () => onLens?.call(GraphLens.speed)),
              const SizedBox(width: 12),
              _button(t,
                  keyName: 'graph-autofit',
                  label: l10n.graphAutoFit,
                  tip: autoFit ? l10n.tipAutoFitOn : l10n.tipAutoFitOff,
                  on: autoFit,
                  onPressed: () => onToggleAutoFit?.call()),
              const SizedBox(width: 12),
            ],
          ],
        ),
      ),
    );
  }
}

/// The outline's end of the bottom bar (§12A.1): one kicker per
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

  /// What the chrome says, read once by the panel and handed down.
  /// These three toggles are the setting's **first consumer**: in every mode
  /// but [ChromeLabels.words] they draw the set's own Switches, Modes and
  /// Parent glyphs, and the word arrives in the tooltip as it does everywhere.
  final ChromeLabels labels;

  /// The Animated filter, which belongs on this end of the bar because it is
  /// the same kind of statement as the toggles beside it: what the **outline**
  /// draws. Everything right of the strip's rule is the document's.
  final bool animatedOnly;
  final VoidCallback onToggleAnimated;

  const ColumnToggles({
    super.key,
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
  /// state from (no bridge call in a build).
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
            // The Animated filter, last of the outline's own marks (6.43). One
            // toggle rather than the withdrawn Keys sheet's *Show — All /
            // Animated* pair: two states are two states, and a word that reads
            // like its neighbours on a strip of them says which is in force
            // without spending a row on saying so twice.
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

/// The set's own glyph for a column group — the one the drawing gives
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
