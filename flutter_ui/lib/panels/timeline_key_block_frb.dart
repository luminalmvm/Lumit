// The Timeline's block-selection box: the box round the caught keys, its
// stretch handles and badge, and the painter that draws a lane's keys.
//
// Split out of timeline_panel_frb.dart.

import 'package:flutter/foundation.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/drag_escape.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'key_block.dart';
import 'timeline_extras_frb.dart';
import 'layer_fold_frb.dart';
import 'timeline_snap.dart';
import 'timeline_metrics_frb.dart';
import 'timeline_lane_area_frb.dart';

/// The block-selection box's metrics, from the approved Keys drawing (K-458).
///
/// The box stands 4px inside its lane top and bottom — 14 of the drawing's
/// 22px row — so a block that covers three rows reads as one region with the
/// seams still visible through it. Its handles are the drawing's 3×6 marks,
/// centred on each end; the badge sits [_blockBadgeGap] past the right one.
const double _blockBoxInset = 4;
const double _blockHandleWidth = 3;
const double _blockHandleHeight = 6;
const double _blockBadgeGap = 6;

/// How wide a stretch handle's *hit target* is, against the 3px it draws.
///
/// K-452's floor bends inside an 18px row but never to three pixels: a mark
/// that thin is a thing to see, not a thing to aim at. Eleven is the width of
/// the key it sits beside, so the two are grabbed with the same accuracy.
const double _blockHandleGrab = 11;
/// The **block-selection box**: the box round everything the marquee caught, a
/// stretch handle at each end, and the badge saying how much it holds
/// (K-458, the approved Keys drawing).
///
/// In plain terms: pick several keyframes and they stop being several things
/// and become one. The box says where they reach, the badge counts them and
/// says how many frames they span, and the handles at its ends let the whole
/// run be squeezed or spread in time — the end you did not touch stays put,
/// and every key keeps its share of the span. The badge is also the way in to
/// the Ease popover, because the drawing anchors that popover to the selection
/// and the badge is the only part of the box that is a control.
///
/// **Shared, not Keys-only.** It is drawn by the lane area, which is the same
/// widget in Layers mode and in Keys mode, so Layers gains the block tools by
/// the same code rather than by a second copy of it (K-441, K-458).
class KeyBlockOverlay extends StatefulWidget {
  /// The selected keys, top to bottom, from the area's one walk.
  final List<SelectedKey> places;
  final TimelineAxis axis;
  final ValueNotifier<KeyStretch?> stretch;
  final bool magnet;

  /// Everything the stretched end can land on — the area's one gathered list,
  /// the same one a lane key's drag snaps against (§4.3, docs/07 §4.5). A
  /// handle used to reach whole frames and nothing else, so a block could not
  /// be pulled onto the marker or the playhead it was being aligned to.
  final List<SnapTarget> snapTargets;
  final int fpsNum;
  final int fpsDen;
  final ProjectReference? project;
  final ValueChanged<Offset> onEase;
  final VoidCallback onChanged;

  /// A click on a handle falls through to the key beneath it: a handle stands
  /// exactly over the block's first and last key, and a control that took the
  /// click and did nothing with it would cost those two keys the ordinary
  /// gestures every other key answers (P5, §2.1).
  final void Function(String id, bool additive) onSelectKey;
  final void Function(String id, Offset position) onKeyMenu;

  const KeyBlockOverlay({super.key, 
    required this.places,
    required this.axis,
    required this.stretch,
    required this.magnet,
    required this.snapTargets,
    required this.fpsNum,
    required this.fpsDen,
    required this.project,
    required this.onEase,
    required this.onChanged,
    required this.onSelectKey,
    required this.onKeyMenu,
  });

  /// What the selection measures. Null when there is no block — fewer than two
  /// keys is a key, and a key has its own drag.
  static KeyBlock? blockOf(List<SelectedKey> places) {
    if (!KeyBlock.isBlock(places.length)) return null;
    var first = places.first.frame;
    var last = places.first.frame;
    for (final p in places) {
      if (p.frame < first) first = p.frame;
      if (p.frame > last) last = p.frame;
    }
    return KeyBlock(first: first, last: last, count: places.length);
  }

  @override
  State<KeyBlockOverlay> createState() => _KeyBlockOverlayState();
}

class _KeyBlockOverlayState extends State<KeyBlockOverlay> {
  /// Where the pointer has put the dragged end, in frames, **before the snap**.
  ///
  /// Kept apart from the stretch's own `to` for the reason every drag here
  /// keeps a running total: adding each event's travel to the *snapped* answer
  /// would make the snap sticky — a caught end would drag the next event's
  /// delta out of the target it had just landed on and back into it, so a
  /// block could not be pulled off a target it had passed.
  double _rawTo = 0;

  /// What the stretch last landed on, so the capture can be drawn — the same
  /// indication a lane key's drag gives (docs/07 §4.5).
  SnapTarget? _caught;

  /// The frames the block's own keys sit on, gathered when the handle was
  /// taken hold of: they all move, so none of them is a place to land.
  Set<double> _moving = const {};

  /// Which end is in hand, so the live readout rides beside it.
  bool _draggingStart = false;

  /// `Escape` abandons the stretch: the box goes back over the keys it started
  /// on and nothing is written (P3, §4.3).
  final DragEscape _escape = DragEscape();

  @override
  void dispose() {
    _escape.dispose();
    super.dispose();
  }

  /// Whether a drag lands on whole frames — the magnet, suspended while Ctrl
  /// is held, exactly as a single key's drag reads it (docs/07 §4.5).
  bool get _whole =>
      widget.magnet &&
      !snapSuspended(
          controlPressed: HardwareKeyboard.instance.isControlPressed);

  /// Commit a finished stretch: every touched row re-timed, the whole set one
  /// undo step (K-458).
  ///
  /// Grouped by row before anything is written, so a row's keys move together
  /// and the strictly-ascending check inside [moveLaneKeys] sees the finished
  /// list rather than one key at a time.
  void commit(KeyStretch moved) {
    if (commitKeyGesture(
      places: widget.places,
      moved: moved,
      whole: _whole,
      fpsNum: widget.fpsNum,
      fpsDen: widget.fpsDen,
      project: widget.project,
    )) {
      widget.onChanged();
    }
  }

  /// Put the block back where the stretch found it and write nothing.
  void _abandon() {
    widget.stretch.value = null;
    if (!mounted) return;
    setState(() => _caught = null);
  }

  /// The id of the key the handle at the block's [start] (or last) end is
  /// standing over at [areaY] — what a click on the handle falls through to.
  ///
  /// Null where the handle covers a row with no selected key at that frame,
  /// which is every row the block merely spans: the box reaches from the top
  /// row to the bottom one whether or not the rows between it hold anything
  /// at that end. The block's resting frames, not the box's live ones — a tap
  /// is a gesture that never became a stretch.
  String? _keyUnderHandle({required bool start, required double areaY}) {
    final block = KeyBlockOverlay.blockOf(widget.places);
    if (block == null) return null;
    final frame = start ? block.first : block.last;
    for (final p in widget.places) {
      if (areaY < p.top || areaY >= p.top + p.height) continue;
      if ((p.frame - frame).abs() > 1e-9) continue;
      return '${p.rowId}#${p.index}';
    }
    return null;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    if (KeyBlockOverlay.blockOf(widget.places) == null) {
      return const SizedBox.shrink();
    }
    return ValueListenableBuilder<KeyStretch?>(
      valueListenable: widget.stretch,
      builder: (context, live, _) {
        final block = KeyBlockOverlay.blockOf(widget.places);
        if (block == null) return const SizedBox.shrink();
        // While a stretch is in flight the box follows it, so the badge counts
        // the span the release will actually write rather than the one the
        // gesture started from.
        double frameOf(double f) =>
            live == null ? f : live.frameOf(f, whole: _whole);
        var top = widget.places.first.top;
        var bottom = widget.places.first.top + widget.places.first.height;
        for (final p in widget.places) {
          if (p.top < top) top = p.top;
          if (p.top + p.height > bottom) bottom = p.top + p.height;
        }
        final first = frameOf(block.first);
        final last = frameOf(block.last);
        final left = widget.axis.xOf(first);
        final right = widget.axis.xOf(last);
        final boxTop = top + _blockBoxInset;
        final boxBottom = bottom - _blockBoxInset;
        final caught = _caught;
        return Stack(
          children: [
            // What the stretch landed on, marked while it holds it — the same
            // capture a lane key's drag draws, because it is the same service
            // (docs/07 §4.5).
            if (live != null && caught != null)
              Positioned(
                key: const ValueKey('tl-block-snap-caught'),
                left: widget.axis.xOf(caught.frame) - 0.5,
                top: boxTop,
                height: boxBottom - boxTop,
                width: 1,
                child: IgnorePointer(child: ColoredBox(color: t.accent)),
              ),
            // The box ignores pointers: it covers the very keys it holds, and
            // one that ate their clicks would make a selected key the one key
            // that cannot be picked up again.
            Positioned(
              key: const ValueKey('tl-block-box'),
              left: left,
              top: boxTop,
              width: right - left,
              height: boxBottom - boxTop,
              child: IgnorePointer(
                child: DecoratedBox(
                  decoration: BoxDecoration(
                    border: Border.all(color: t.textPrimary),
                  ),
                ),
              ),
            ),
            _handle(t, x: left, top: boxTop, bottom: boxBottom, start: true),
            _handle(t, x: right, top: boxTop, bottom: boxBottom, start: false),
            // Measured from where the box is *now*, so the span it reports is
            // the one the release will write rather than the one the gesture
            // started from.
            _badge(t, KeyBlock(first: first, last: last, count: block.count),
                right: right, top: boxTop),
            // The stretch's own live readout, under the hand and gone on
            // release (§4.2, P1). Not for a **move**: the lane the hand is on
            // is already showing the frame the key has reached, and a second
            // pill saying the same thing in other words is two readouts for one
            // gesture.
            if (live != null && live.shift == 0)
              _stretchHint(first, last,
                  left: left, right: right, bottom: boxBottom),
          ],
        );
      },
    );
  }

  /// One end of the box: the drawing's 3x6 mark, inside a hit target wide
  /// enough to aim at (K-452).
  ///
  /// [start] is the earlier end, and dragging it anchors the *later* one — the
  /// end you are not holding is the end that stays put, which is the whole of
  /// what makes a stretch feel like a stretch rather than a move.
  Widget _handle(
    LumitTheme t, {
    required double x,
    required double top,
    required double bottom,
    required bool start,
  }) =>
      Positioned(
        key: ValueKey<String>('tl-block-handle-${start ? 'start' : 'end'}'),
        left: x - _blockHandleGrab / 2,
        top: top,
        width: _blockHandleGrab,
        height: bottom - top,
        child: MouseRegion(
          cursor: SystemMouseCursors.resizeLeftRight,
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            supportedDevices: dragDevices,
            // **From the down, not from the slop.** A handle that also answers
            // a tap has a tap recogniser beside its drag one, and a drag that
            // has to out-wait a competitor starts where the slop was passed —
            // which leaves the mark a pointer's width behind the cursor for the
            // rest of the gesture. Taken from the down, the mark stays exactly
            // under the hand, which is what a precision handle is for.
            dragStartBehavior: DragStartBehavior.down,
            // **The click falls through to the key beneath** (§2.1). The
            // handle stands exactly over the block's end key and is opaque, so
            // without this the two keys a block cares about most were the only
            // two that could not be clicked or right-clicked at all.
            onTapUp: (d) {
              final id = _keyUnderHandle(
                  start: start, areaY: top + d.localPosition.dy);
              if (id == null) return;
              final keys = HardwareKeyboard.instance;
              widget.onSelectKey(
                  id,
                  keys.isShiftPressed ||
                      keys.isControlPressed ||
                      keys.isMetaPressed);
            },
            onSecondaryTapUp: (d) {
              final id = _keyUnderHandle(
                  start: start, areaY: top + d.localPosition.dy);
              if (id != null) widget.onKeyMenu(id, d.globalPosition);
            },
            onHorizontalDragStart: (_) {
              final block = KeyBlockOverlay.blockOf(widget.places);
              if (block == null) return;
              final from = start ? block.first : block.last;
              final anchor = start ? block.last : block.first;
              _rawTo = from;
              _draggingStart = start;
              // Every key in hand moves, so none of them is a place to land.
              _moving = {for (final p in widget.places) p.frame};
              widget.stretch.value = KeyStretch(
                keys: {for (final p in widget.places) '${p.rowId}#${p.index}'},
                anchor: anchor,
                from: from,
                to: from,
              );
              _escape.begin(_abandon);
            },
            onHorizontalDragUpdate: (d) {
              final held = widget.stretch.value;
              if (held == null ||
                  widget.axis.perFrame <= 0 ||
                  !_escape.running) {
                return;
              }
              // The pointer's own answer first, held inside the block's bounds,
              // and the snap taken from that — so a caught end lets go again as
              // soon as the pointer has travelled past the target.
              _rawTo = clampStretch(
                anchor: held.anchor,
                from: held.from,
                to: _rawTo + d.delta.dx / widget.axis.perFrame,
              );
              final snapped = snapFrame(
                frame: _rawTo,
                targets: widget.snapTargets.where((s) =>
                    s.kind != SnapKind.keyframe || !_moving.contains(s.frame)),
                perFrame: widget.axis.perFrame,
                magnet: _whole,
              );
              setState(() => _caught = snapped.caught);
              widget.stretch.value = held.movedTo(clampStretch(
                anchor: held.anchor,
                from: held.from,
                to: snapped.frame,
              ));
            },
            onHorizontalDragEnd: (_) {
              final held = widget.stretch.value;
              widget.stretch.value = null;
              if (mounted) setState(() => _caught = null);
              // Nothing to commit when `Escape` already took the gesture: it
              // put the block back, and put back is put back whatever the
              // pointer did afterwards.
              if (_escape.end() && held != null) commit(held);
            },
            onHorizontalDragCancel: () {
              _escape.end();
              _abandon();
            },
            child: Center(
              child: SizedBox(
                width: _blockHandleWidth,
                height: _blockHandleHeight,
                child: ColoredBox(color: t.textPrimary),
              ),
            ),
          ),
        ),
      );

  /// The badge: how many keys the block holds and how many frames it spans,
  /// and — pressed — the way into the Ease popover (K-458).
  Widget _badge(
    LumitTheme t,
    KeyBlock block, {
    required double right,
    required double top,
  }) =>
      Positioned(
        left: right + _blockBadgeGap,
        // Level with the top of the box, less the pixel of padding the badge
        // wears, so the two read as one mark rather than as a label that has
        // slipped.
        top: top - 1,
        child: Builder(
          builder: (badgeContext) => LumitTooltip(
            message: l10n.tipEaseTheBlock,
            child: GestureDetector(
              key: const ValueKey('tl-block-badge'),
              behavior: HitTestBehavior.opaque,
              onTap: () {
                final box = badgeContext.findRenderObject() as RenderBox?;
                if (box == null) return;
                widget.onEase(box.localToGlobal(Offset.zero));
              },
              child: Container(
                padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
                decoration: BoxDecoration(
                  // The drawing's own 2b3034 — the ramp's top step, which is
                  // what lifts a small label off the black lane ground it sits
                  // on without giving it a border to carry.
                  color: t.surface4,
                  borderRadius: BorderRadius.circular(2),
                ),
                child: Text(
                  l10n.keyBlockBadge(block.count, block.spanFrames),
                  style: t.mono.copyWith(fontSize: 8, color: t.textPrimary),
                ),
              ),
            ),
          ),
        ),
      );

  /// The live readout a stretch summons: where the block's two ends have
  /// reached, under the hand and gone on release (§4.2, P1).
  ///
  /// Frames only — the badge above says how many keys and how wide, and a
  /// block of many keys has no one value to report. It rides at the *dragged*
  /// end, below the box, which is where the pointer is and clear of the badge
  /// at the top right.
  Widget _stretchHint(
    double first,
    double last, {
    required double left,
    required double right,
    required double bottom,
  }) {
    final x = _draggingStart ? left : right;
    // Beside the handle, or on its other side where the axis has run out.
    const pill = 76.0;
    return Positioned(
      key: const ValueKey('tl-block-stretch-hint'),
      left: x + 8 + pill > widget.axis.width ? x - 8 - pill : x + 8,
      top: bottom + 2,
      child: HintPill(
        text: l10n.timelineStretchHint(first.round(), last.round()),
      ),
    );
  }
}

/// A lane's keyframe diamonds: one per key, in `animated` (§3.1) — the token
/// that means "this is animated or in hand" — and `text_primary` for the ones
/// the marquee has hold of. Neither is `accent`: the accent's job list is the
/// playhead, the one filled button and the active tab tick, and nothing else.
class LaneKeysPainter extends CustomPainter {
  /// Fractional, so a key placed between frames draws between them.
  final List<double> frames;
  final Set<int> selected;
  final TimelineAxis axis;
  final Color colour;
  final Color chosen;

  /// Half a key's height. [laneKeyHalf] on a property's own lane in either
  /// mode; half of that on a shut layer's row, where the marks are a summary
  /// rather than the things you drag (§12A.1).
  final double half;

  /// Each key's two halves — the shape of the interpolation coming in and the
  /// one going out (K-457) — or null on a shut layer's summary row, where a
  /// plain diamond is all a mark that cannot be aimed at has to say.
  final List<(KeyShape, KeyShape)>? shapes;

  /// The key under the pointer, drawn **half way** from [colour] to [chosen]
  /// (§4.2): far enough to answer the hand, short of the mark a selected key
  /// carries, so a hovered key is never mistaken for a caught one. Null on a
  /// summary row, whose marks are a statement rather than a target (K-441).
  final int? hovered;

  const LaneKeysPainter({
    required this.frames,
    required this.selected,
    required this.axis,
    required this.colour,
    required this.chosen,
    this.half = laneKeyHalf,
    this.shapes,
    this.hovered,
  });

  @override
  void paint(Canvas canvas, Size size) {
    final mid = size.height / 2;
    for (var i = 0; i < frames.length; i++) {
      final x = axis.xOf(frames[i]);
      final paint = Paint()
        ..color = selected.contains(i)
            ? chosen
            : i == hovered
                ? Color.lerp(colour, chosen, 0.5)!
                : colour;
      final (into, out) = shapes == null || i >= shapes!.length
          ? (KeyShape.diamond, KeyShape.diamond)
          : shapes![i];
      // One path, one call: two anti-aliased halves meeting on the centre line
      // left a seam down the middle of every mark (§5).
      canvas.drawPath(keyMarkPath((into, out), x, mid, half), paint);
    }
  }

  @override
  bool shouldRepaint(LaneKeysPainter old) =>
      !listEquals(old.frames, frames) ||
      !setEquals(old.selected, selected) ||
      old.colour != colour ||
      old.chosen != chosen ||
      old.half != half ||
      old.hovered != hovered ||
      !listEquals(old.shapes, shapes) ||
      old.axis.frames != axis.frames ||
      old.axis.width != axis.width;

  /// A background painter's default is to absorb hits across its whole rect,
  /// which would eat the keyframe marquee underneath (the diamonds are picked
  /// up by the box, not clicked).
  @override
  bool? hitTest(Offset position) => false;
}
