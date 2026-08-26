// The Timeline lanes' bottom bar: zoom out / in / fit, the magnet, the
// horizontal scrollbar, and the graph's own commands in graph view.
//
// Split out of timeline_panel_frb.dart.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'graph_editor_frb.dart';
import 'graph_maths.dart';
import '../widgets/smooth_zoom.dart';
import 'timeline_metrics_frb.dart';
import 'timeline_outline_frb.dart';

/// The two landscapes flanking the zoom slider (K-293). Painter-drawn, so
/// K-209's 16px floor — which is about an icon-set glyph's 1.5-unit stroke
/// falling on less than a pixel — does not apply: a filled shape has no stroke
/// to lose. They sit inside a 20px bar, and the pair has to differ plainly
/// enough to read as "less of this / more of this" at a glance.
const double _zoomGlyphSmall = 9;
const double _zoomGlyphLarge = 14;
/// The lanes' bottom bar (docs/07 §4.5-§4.6): − / + / Fit with the zoom read
/// out, the magnet, and the horizontal scrollbar that moves the zoomed view.
///
/// In graph view it also carries the graph's own commands (docs/07 §5.3):
/// Linear / Bezier / Hold for the selected keys, the value/speed lens
/// switch, and the auto-fit toggle.
///
/// A panel bottom bar, and so a **secondary row**: `t.density.secondaryRow`
/// (K-451, K-454). The outline reserves the same height below its own rows —
/// see `_outlineHalf`, where the reason is written down.
class LaneBottomBar extends StatelessWidget {
  /// Where the zoom is *going*, not where the flight has reached — so the
  /// handle sits under the finger that put it there rather than trailing the
  /// animation by a flight's length (K-293).
  final double zoom;

  /// The far end of the slider: the zoom at which the lanes show
  /// `_TimelinePanelFrbState._framesAtFullZoom` frames.
  final double maxZoom;
  final ScrollController hScroll;

  /// A zoom asked for in one step — a tap on the track — which flies.
  final ValueChanged<double> onZoom;

  /// A zoom asked for continuously, while the handle is dragged. The drag is
  /// the motion, so this one arrives at once.
  final ValueChanged<double> onZoomLive;

  /// The drag's ends, so the panel can anchor once per gesture (K-319).
  final VoidCallback? onZoomDragStart;
  final VoidCallback? onZoomDragEnd;
  final bool magnet;
  final VoidCallback onToggleMagnet;

  /// Set in graph view; null hides the graph commands (the lane view).
  final GraphLens? lens;
  final ValueChanged<GraphLens>? onLens;
  final bool autoFit;
  final VoidCallback? onToggleAutoFit;
  final ValueChanged<BridgeSideInterp>? onInterp;

  /// A tangent mode chosen for the selected keys — Auto / Clamp / Free (§6.3).
  final ValueChanged<TangentMode>? onTangentMode;

  /// The Easing… button pressed, with the button's own context so a popup can
  /// be anchored to it. Whether that is a popup or a docked panel is the
  /// panel's decision, not this bar's (K-349).
  final ValueChanged<BuildContext>? onOpenEasing;

  /// Set in **Layers mode**: the keyframe strip — Interpolation (Linear /
  /// Hold / Ease / Bezier), then Reverse, Copy and Paste at playhead (K-458).
  ///
  /// It was Keys mode's, because the approved Keys drawing is where it was
  /// drawn. Keys mode is gone (K-529) and the strip is the part of it the
  /// owner values, so it moved to the lane bar rather than going with the
  /// mode: the same seven commands act on the same key selection whichever
  /// list the keys were picked from.
  final bool strip;

  /// The Ease word pressed, with its own context so the popover can be
  /// anchored to it — the same box the block badge opens.
  final ValueChanged<BuildContext>? onEaseBlock;
  final VoidCallback? onReverse;
  final VoidCallback? onCopy;
  final VoidCallback? onPaste;

  const LaneBottomBar({super.key, 
    required this.zoom,
    required this.maxZoom,
    required this.hScroll,
    required this.onZoom,
    required this.onZoomLive,
    this.onZoomDragStart,
    this.onZoomDragEnd,
    required this.magnet,
    required this.onToggleMagnet,
    this.lens,
    this.onLens,
    this.autoFit = true,
    this.onToggleAutoFit,
    this.onInterp,
    this.onTangentMode,
    this.onOpenEasing,
    this.strip = false,
    this.onEaseBlock,
    this.onReverse,
    this.onCopy,
    this.onPaste,
  });

  Widget _graphButton(
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
          // 18px bottom bar (K-451): one pixel of the button's own edge is
          // all the room a 9px kicker leaves above and below it.
          padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 1),
          onPressed: onPressed,
          child: Text(label.toUpperCase(), style: on ? t.kickerOn : t.kicker),
        ),
      );

  /// The zoom slider between its two landscapes, and the magnet — the run this
  /// bar carries in **every** view, at the left edge of the lane area in every
  /// view (owner, desktop testing).
  ///
  /// The zoom is a slider between a small landscape and a large one (owner,
  /// 2026-08-06) — the pair After Effects flanks its own zoom slider with. The
  /// far left is the whole composition; the far right is twenty frames across
  /// the lanes, whatever the comp's length. It replaced − / + / Fit: the two
  /// ends *are* Fit and full zoom, and a slider says where you are between
  /// them, which three buttons never did.
  ///
  /// Painter-drawn and small, both deliberately: the pair only says
  /// "less / more" if the sizes plainly differ, and a stroked glyph under 16px
  /// crunches (K-209), so these are filled shapes with no stroke to lose.
  /// One end of the slider: the landscape, and **a click on it nudges the
  /// zoom one step** (§6.5). The pair had been decoration — a picture of what
  /// the two ends mean — while every other icon in this bar does something,
  /// and a step at a time is exactly what a slider is bad at.
  ///
  /// The glyph keeps its drawn size and its place; only the height of what
  /// takes the click grows, to the bar's own, so aiming at the smaller of the
  /// two is not a test of aim. The step is the keys' step, so the slider's
  /// ends and `=` / `-` cannot disagree.
  Widget _zoomEnd(LumitTheme t, {required bool inward}) {
    final size = inward ? _zoomGlyphLarge : _zoomGlyphSmall;
    return LumitTooltip(
      message: inward ? l10n.tipZoomIn : l10n.tipZoomOut,
      child: MouseRegion(
        cursor: SystemMouseCursors.click,
        child: GestureDetector(
          key: ValueKey<String>('tl-zoom-${inward ? 'in' : 'out'}'),
          behavior: HitTestBehavior.opaque,
          onTap: () =>
              onZoom(zoomNudged(zoom, inward: inward, maxZoom: maxZoom)),
          child: SizedBox(
            width: size,
            height: t.density.secondaryRow,
            child: Center(
              child: lumitIcon(LumitIcon.zoomExtent,
                  size: size, color: t.textMuted),
            ),
          ),
        ),
      ),
    );
  }

  List<Widget> _zoomAndMagnet(LumitTheme t) => [
        _zoomEnd(t, inward: false),
        const SizedBox(width: 4),
        LumitTooltip(
          message: l10n.tipZoomPercent('${(zoom * 100).round()}'),
          child: HouseSlider(
            key: const ValueKey('tl-zoom-slider'),
            // The slider runs on the *logarithm* of the zoom, so equal travel
            // buys equal ratio — the same reason the flight interpolates that
            // way. A linear one would spend nine tenths of its length in the
            // last few frames of a long comp.
            value: zoomSliderPosition(zoom, maxZoom),
            min: 0,
            max: 1,
            width: 96,
            showValue: false,
            // Dragged, the zoom follows the finger with no flight; tapped, it
            // flies to where the track was clicked (K-293). The drag's ends
            // bracket the gesture so the panel anchors once (K-319).
            onChangeStart: onZoomDragStart,
            onChangeEnd: onZoomDragEnd,
            onChangeLive: (v) => onZoomLive(zoomForSliderPosition(v, maxZoom)),
            onChanged: (v) => onZoom(zoomForSliderPosition(v, maxZoom)),
          ),
        ),
        const SizedBox(width: 4),
        _zoomEnd(t, inward: true),
        const SizedBox(width: 6),
        LumitTooltip(
          message: magnet ? l10n.tipSnapOn : l10n.tipSnapOff,
          child: HouseButton(
            key: const ValueKey('tl-magnet'),
            small: true,
            // **No accent**: §3.1's list is closed — the one filled button, the
            // playhead, the workspace tick — and a snap toggle is not on it. On
            // reads the way every other toggle in this chrome reads: the glyph
            // at foreground strength on the button's own face, off is frameless
            // and muted.
            frameless: !magnet,
            padding: const EdgeInsets.symmetric(horizontal: 4),
            onPressed: onToggleMagnet,
            child: lumitIcon(LumitIcon.magnet,
                size: iconSize, color: magnet ? t.textPrimary : t.textMuted),
          ),
        ),
        const SizedBox(width: 12),
      ];

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      // Keyed so the density tests have a secondary row they can measure
      // whole; every other one is a strip with no handle on it (K-454).
      key: const ValueKey('tl-lane-bottom-bar'),
      height: t.density.secondaryRow,
      // A panel bottom bar, and so `surface_2` — the same value the panel
      // header wears at the other end of the panel (K-451).
      color: t.surface2,
      padding: const EdgeInsets.symmetric(horizontal: 10),
      child: LayoutBuilder(
        builder: (context, constraints) {
          // The buttons scroll sideways when the panel is narrow — the same
          // answer the Timeline toolbar gives; an overflow stripe is a
          // layout fault. The scrollbar keeps its share of the bar whatever
          // the buttons need.
          final buttonRoom =
              (constraints.maxWidth - 120).clamp(0.0, constraints.maxWidth);
          return Row(
            children: [
              ConstrainedBox(
                constraints: BoxConstraints(maxWidth: buttonRoom),
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: Row(
                    children: [
                      // **The zoom and the magnet come first, in every view**
                      // (owner, desktop testing). They are the one run this
                      // bar carries whatever the panel is showing, so they
                      // are the run that must not move: in graph view they
                      // sat behind four runs of graph commands, which put the
                      // same two controls in a different place depending on
                      // which mode was up. At the left edge of the lane area
                      // they are where the eye already is.
                      ..._zoomAndMagnet(t),
                      // The keyframe strip (K-458), in Layers mode. A label,
                      // then the four interpolations, then a rule, then the
                      // three commands — the same 2-inside-a-run,
                      // 12-between-runs rhythm the graph's buttons keep.
                      if (strip) ...[
                        Text(l10n.fxInterpolation.toUpperCase(),
                            style: t.kicker),
                        const SizedBox(width: 8),
                        _graphButton(t,
                            keyName: 'keys-interp-linear',
                            label: l10n.easeLinear,
                            tip: l10n.tipLinearKeyframes,
                            on: false,
                            onPressed: () => onInterp
                                ?.call(const BridgeSideInterp.linear())),
                        const SizedBox(width: 2),
                        _graphButton(t,
                            keyName: 'keys-interp-hold',
                            label: l10n.easeHold,
                            tip: l10n.tipHoldKeyframes,
                            on: false,
                            onPressed: () =>
                                onInterp?.call(const BridgeSideInterp.hold())),
                        const SizedBox(width: 2),
                        // The shaped ease, one step along from the flat three:
                        // the same selection, a curve chosen by name instead of
                        // a constant. Its own Builder so the popover can be
                        // anchored to the word that opened it.
                        Builder(
                          builder: (buttonContext) => _graphButton(t,
                              keyName: 'keys-interp-ease',
                              label: l10n.keysEase,
                              tip: l10n.tipEaseTheBlock,
                              on: false,
                              onPressed: () =>
                                  onEaseBlock?.call(buttonContext)),
                        ),
                        const SizedBox(width: 2),
                        _graphButton(t,
                            keyName: 'keys-interp-bezier',
                            label: l10n.easeBezier,
                            tip: l10n.tipEasyEase,
                            on: false,
                            onPressed: () => onInterp?.call(easyEase)),
                        const SizedBox(width: 8),
                        // **The two runs read apart**: everything left of this
                        // rule says how the movement between keys is shaped;
                        // everything right of it moves the keys themselves.
                        Container(
                            width: 1, height: 10, color: t.hairlineStrong),
                        const SizedBox(width: 8),
                        _graphButton(t,
                            keyName: 'keys-reverse',
                            label: l10n.fxReverse,
                            tip: l10n.tipReverseKeys,
                            on: false,
                            onPressed: () => onReverse?.call()),
                        const SizedBox(width: 2),
                        _graphButton(t,
                            keyName: 'keys-copy',
                            label: l10n.menuCopy,
                            tip: l10n.tipCopyKeys,
                            on: false,
                            onPressed: () => onCopy?.call()),
                        const SizedBox(width: 2),
                        _graphButton(t,
                            keyName: 'keys-paste',
                            label: l10n.keysPasteAtPlayhead,
                            tip: l10n.tipPasteKeysAtPlayhead,
                            on: false,
                            onPressed: () => onPaste?.call()),
                        const SizedBox(width: 12),
                      ],
                      if (lens != null) ...[
                        // The selected keys' easing, one click each — the F9 family's
                        // buttons (docs/07 §5.3).
                        //
                        // Two gaps run through this bar: 2 between the chips
                        // of one segmented run, 12 between one run and the
                        // next, so the runs read as groups rather than as one
                        // long strip of buttons.
                        _graphButton(t,
                            keyName: 'graph-interp-linear',
                            label: l10n.easeLinear,
                            tip: l10n.tipLinearKeyframes,
                            on: false,
                            onPressed: () => onInterp
                                ?.call(const BridgeSideInterp.linear())),
                        const SizedBox(width: 2),
                        _graphButton(t,
                            keyName: 'graph-interp-bezier',
                            label: l10n.easeBezier,
                            tip: l10n.tipEasyEase,
                            on: false,
                            onPressed: () => onInterp?.call(easyEase)),
                        const SizedBox(width: 2),
                        _graphButton(t,
                            keyName: 'graph-interp-hold',
                            label: l10n.easeHold,
                            tip: l10n.tipHoldKeyframes,
                            on: false,
                            onPressed: () =>
                                onInterp?.call(const BridgeSideInterp.hold())),
                        // The shaped ease, one step along from the one-click
                        // three: same selection, a curve instead of a constant.
                        // Its own Builder so the popup can find where this
                        // button is; the popup layout slides it up into view.
                        //
                        // Value lens only. The box draws a shape against the
                        // value's own travel, so a curve stamped while the
                        // speed lens is up would land on the value graph — a
                        // change the user cannot see in the view they drew it
                        // in. The one-click three above stay in both lenses: a
                        // side's interp means the same thing either way.
                        if (lens == GraphLens.value) ...[
                          const SizedBox(width: 2),
                          Builder(
                            builder: (buttonContext) => _graphButton(t,
                                keyName: 'graph-interp-easing',
                                label: l10n.easeCustom,
                                tip: l10n.tipEasingEditor,
                                on: false,
                                onPressed: () =>
                                    onOpenEasing?.call(buttonContext)),
                          ),
                        ],
                        const SizedBox(width: 12),
                        // Tangents — Auto / Clamp / Free (§6.3), between the
                        // ease presets and the lens pair. A run of three like
                        // the eases beside them, and unlit for the same
                        // reason: these are things to *do* to the selection,
                        // and a selection spanning two modes has no one answer
                        // to light. Which mode a side is in is legible where
                        // it matters — in the handle, which stops following
                        // its neighbours the moment it is dragged.
                        _graphButton(t,
                            keyName: 'graph-tangent-auto',
                            label: l10n.graphTangentAuto,
                            tip: l10n.tipTangentAuto,
                            on: false,
                            onPressed: () =>
                                onTangentMode?.call(TangentMode.auto)),
                        const SizedBox(width: 2),
                        _graphButton(t,
                            keyName: 'graph-tangent-clamp',
                            label: l10n.graphTangentClamp,
                            tip: l10n.tipTangentClamp,
                            on: false,
                            onPressed: () =>
                                onTangentMode?.call(TangentMode.clamp)),
                        const SizedBox(width: 2),
                        _graphButton(t,
                            keyName: 'graph-tangent-free',
                            label: l10n.graphTangentFree,
                            tip: l10n.tipTangentFree,
                            on: false,
                            onPressed: () =>
                                onTangentMode?.call(TangentMode.free)),
                        const SizedBox(width: 12),
                        _graphButton(t,
                            keyName: 'graph-lens-value',
                            label: l10n.clipboardValueColumn,
                            tip: l10n.tipValueGraph,
                            on: lens == GraphLens.value,
                            onPressed: () => onLens?.call(GraphLens.value)),
                        const SizedBox(width: 2),
                        _graphButton(t,
                            keyName: 'graph-lens-speed',
                            label: l10n.graphSpeed,
                            tip: l10n.tipSpeedGraph,
                            on: lens == GraphLens.speed,
                            onPressed: () => onLens?.call(GraphLens.speed)),
                        const SizedBox(width: 12),
                        _graphButton(t,
                            keyName: 'graph-autofit',
                            label: l10n.graphAutoFit,
                            tip: autoFit
                                ? l10n.tipAutoFitOn
                                : l10n.tipAutoFitOff,
                            on: autoFit,
                            onPressed: () => onToggleAutoFit?.call()),
                        const SizedBox(width: 12),
                      ],
                    ],
                  ),
                ),
              ),
              Expanded(
                child: GutterScrollbar(
                  controller: hScroll,
                  axis: Axis.horizontal,
                ),
              ),
            ],
          );
        },
      ),
    );
  }
}
