// The Timeline lanes' bottom bar: the zoom slider, the magnet, and the
// horizontal scrollbar.
//
// Split out of timeline_panel_frb.dart.

import 'package:flutter/widgets.dart';
import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/smooth_zoom.dart';
import 'timeline_metrics_frb.dart';
import 'timeline_outline_frb.dart';

/// The two landscapes flanking the zoom slider. Painter-drawn, so the 16px
/// floor — which is about an icon-set glyph's 1.5-unit stroke falling on less
/// than a pixel — does not apply: a filled shape has no stroke to lose. They
/// sit inside a 20px bar, and the pair has to differ plainly enough to read as
/// "less of this / more of this" at a glance.
const double _zoomGlyphSmall = 9;
const double _zoomGlyphLarge = 14;

/// The lanes' bottom bar (docs/07 §4.5-§4.6): the zoom slider between its two
/// landscapes, the magnet, and the horizontal scrollbar that moves the zoomed
/// view — and **nothing else, in either view**.
///
/// The key commands and the graph's own commands used to stand here too, which
/// made the bar under the lanes the longest strip of buttons in the panel while
/// the strip under the outline had room to spare. They moved to the outline's
/// foot (`KeyCommandStrip`), beside the column toggles: they act on what is
/// selected in the outline above them, so that is where the eye already is.
///
/// A panel bottom bar, and so a **secondary row**: `t.density.secondaryRow`.
/// The outline reserves the same height below its own rows —
/// see `_outlineHalf`, where the reason is written down.
class LaneBottomBar extends StatelessWidget {
  /// Where the zoom is *going*, not where the flight has reached — so the
  /// handle sits under the finger that put it there rather than trailing the
  /// animation by a flight's length.
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

  /// The drag's ends, so the panel can anchor once per gesture.
  final VoidCallback? onZoomDragStart;
  final VoidCallback? onZoomDragEnd;
  final bool magnet;
  final VoidCallback onToggleMagnet;

  const LaneBottomBar({
    super.key,
    required this.zoom,
    required this.maxZoom,
    required this.hScroll,
    required this.onZoom,
    required this.onZoomLive,
    this.onZoomDragStart,
    this.onZoomDragEnd,
    required this.magnet,
    required this.onToggleMagnet,
  });

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
  /// crunches, so these are filled shapes with no stroke to lose.
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

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      // Keyed so the density tests have a secondary row they can measure
      // whole; every other one is a strip with no handle on it.
      key: const ValueKey('tl-lane-bottom-bar'),
      height: t.density.secondaryRow,
      // A panel bottom bar, and so `surface_2` — the same value the panel
      // header wears at the other end of the panel.
      color: t.surface2,
      padding: const EdgeInsets.symmetric(horizontal: 10),
      child: Row(
        children: [
          // The run scrolls sideways rather than overflowing when the panel is
          // squeezed — the same answer the toolbar gives; an overflow stripe is
          // a layout fault. Loose, so at any ordinary width it takes exactly
          // what it needs and the scrollbar keeps the rest.
          Flexible(
            child: SingleChildScrollView(
              scrollDirection: Axis.horizontal,
              child: Row(
                children: [
                  _zoomEnd(t, inward: false),
                  const SizedBox(width: 4),
                  LumitTooltip(
                    message: l10n.tipZoomPercent('${(zoom * 100).round()}'),
                    child: HouseSlider(
                      key: const ValueKey('tl-zoom-slider'),
                      // The slider runs on the *logarithm* of the zoom, so
                      // equal travel buys equal ratio — the same reason the
                      // flight interpolates that way. A linear one would spend
                      // nine tenths of its length in the last few frames of a
                      // long comp.
                      value: zoomSliderPosition(zoom, maxZoom),
                      min: 0,
                      max: 1,
                      width: 96,
                      showValue: false,
                      // Dragged, the zoom follows the finger with no flight;
                      // tapped, it flies to where the track was clicked. The
                      // drag's ends bracket the gesture so the panel anchors
                      // once.
                      onChangeStart: onZoomDragStart,
                      onChangeEnd: onZoomDragEnd,
                      onChangeLive: (v) =>
                          onZoomLive(zoomForSliderPosition(v, maxZoom)),
                      onChanged: (v) =>
                          onZoom(zoomForSliderPosition(v, maxZoom)),
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
                      // **No accent**: §3.1's list is closed — the one filled
                      // button, the playhead, the workspace tick — and a snap
                      // toggle is not on it. On reads the way every other
                      // toggle in this chrome reads: the glyph at foreground
                      // strength on the button's own face, off is frameless
                      // and muted.
                      frameless: !magnet,
                      padding: const EdgeInsets.symmetric(horizontal: 4),
                      onPressed: onToggleMagnet,
                      child: lumitIcon(LumitIcon.magnet,
                          size: iconSize,
                          color: magnet ? t.textPrimary : t.textMuted),
                    ),
                  ),
                  const SizedBox(width: 12),
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
      ),
    );
  }
}
