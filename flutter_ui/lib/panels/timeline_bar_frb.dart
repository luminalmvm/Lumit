// One layer's bar in the Timeline, and the geometry a bar drag works in: the
// live preview, the bounds a bar may be dragged within, and the end marks.
//
// Split out of timeline_panel_frb.dart.

import 'dart:math';
import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/drag_escape.dart';
import 'timeline_extras_frb.dart';
import 'layer_fold_frb.dart';
import 'timeline_snap.dart';
import 'timeline_key_block_frb.dart';

/// The same on a **shut layer's** row: smaller than the keys you take hold
/// of, because these are a summary of everything keyed inside the layer.
///
/// **8 point to point, measured off the drawing** (2026-08-24). The mockup's
/// summary mark is a 4px square with a 1px border stood on its corner, which
/// renders 8×8 — not the 5.7 the square's own side had been read as, and not
/// the 5 the outline drew. Half of 8 is the number here. Twirl the layer open
/// and each property draws its own at full size, where they can be dragged.
const double _summaryKeyHalf = 4;
/// How near the end of a bar counts as grabbing its edge to trim rather than its
/// middle to move.
const double _trimGrab = 8;

/// Which part of a bar [width] pixels wide a press at [dx] takes hold of.
///
/// Each trim zone is [_trimGrab] wide but never more than a third of the bar,
/// so a bar only a few frames long still keeps a middle to move by — without
/// the cap, a short bar was all edge and could not be dragged along the
/// timeline at all.
BarGrab barGrabAt(double dx, double width) {
  final edge = min(_trimGrab, width / 3);
  if (dx < edge) return BarGrab.trimIn;
  if (dx > width - edge) return BarGrab.trimOut;
  return BarGrab.move;
}
/// The live preview of a bar drag in flight: how far each edge and the start
/// offset have moved, in frames. Published by the bar and read by the waveform
/// lane, so the transients travel with the bar rather than jumping on release
/// (K-172). Null between gestures.
class BarDragPreview {
  /// The layer whose bar the hand is on.
  final String layerId;

  /// Every layer the drag carries (K-720): [layerId] alone for a trim or a
  /// single-layer move, the whole unlocked selection when a move drag starts
  /// on a selected bar. Each travels by the same three deltas, because only a
  /// move is ever plural and a move's three deltas are one number.
  final Set<String> layers;
  final int deltaIn;
  final int deltaOut;
  final int offsetShift;
  const BarDragPreview(
      this.layerId, this.layers, this.deltaIn, this.deltaOut, this.offsetShift);
}

/// What a grab of [grab] moved by [delta] frames does to a layer's span.
/// Moving carries the content with the bar, so the start offset travels too;
/// a trim leaves the content where it is and moves one edge over it.
/// [moving] is the selection travelling with a move (K-720), the grabbed
/// layer included; trims stay single-layer whatever is selected.
BarDragPreview barDragPreview(String layerId, BarGrab grab, int delta,
        {Set<String>? moving}) =>
    switch (grab) {
      BarGrab.move =>
        BarDragPreview(layerId, moving ?? {layerId}, delta, delta, delta),
      BarGrab.trimIn => BarDragPreview(layerId, {layerId}, delta, 0, 0),
      BarGrab.trimOut => BarDragPreview(layerId, {layerId}, 0, delta, 0),
    };

/// How far a bar drag in flight moves [layerId]'s keyframes, in frames
/// (§6.26).
///
/// The **start offset's** travel, not an edge's: a keyframe's time is the
/// layer's own, carried onto the comp's clock by that offset (K-213), so a
/// move slides every key with the bar and a trim slides none. Answered for
/// **every** layer the drag carries (K-720), so a selection-mate's keys travel
/// with its bar; zero for a layer outside the drag, and zero between gestures.
int keyShiftOf(BarDragPreview? preview, String layerId) =>
    preview != null && preview.layers.contains(layerId)
        ? preview.offsetShift
        : 0;

/// The selection's share of a move drag (K-720): every **unlocked** selected
/// layer — a locked layer sits still, exactly as it refuses its share of a
/// switch batch — and the wall the set stops at.
class SelectionMove {
  /// In stack order; the grabbed layer is among them.
  final List<UuidValue> layerIds;

  /// The earliest in frame among them: the whole set's travel stops where
  /// this one meets comp zero, so the set hits the wall with its shape
  /// intact — the same clamp the group bar's slide has.
  final int minIn;
  const SelectionMove(this.layerIds, this.minIn);
}

/// How far a layer's ends may be dragged, in comp frames (K-211).
///
/// **In plain terms:** a Footage, audio or Precomp layer can only show what its
/// source actually holds, so its bar stops where the media does — its head
/// cannot be dragged earlier than the source's first frame, and its tail cannot
/// be dragged past its last. Every generated kind — Solid, Text, Adjustment,
/// Null, Camera, Sequence — has no such source, so both its ends are free and
/// it is whatever length the user drags it to. Switching **Retime** on frees
/// the ends too (docs/04-RETIMING.md): a retimed layer decides for itself which
/// source moment each of its own frames shows, so its length stops being the
/// source's business.
class BarBounds {
  /// The earliest frame the in point may be trimmed to; null = the head is free.
  final int? minIn;

  /// The latest frame the out point may be trimmed to; null = the tail is free.
  final int? maxOut;

  const BarBounds({this.minIn, this.maxOut});

  /// Both ends free: every generated kind, anything retimed, and any source
  /// whose length could not be read.
  static const BarBounds free = BarBounds();

  @override
  bool operator ==(Object other) =>
      other is BarBounds && other.minIn == minIn && other.maxOut == maxOut;

  @override
  int get hashCode => Object.hash(minIn, maxOut);
}

/// The bounds one layer's bar trims within.
///
/// [startOffsetFrame] is where the layer's own time zero sits on the comp
/// timeline, which is where its source's first frame shows; [sourceFrames] is
/// the source's length in comp frames, or null when the layer has no source of
/// its own — or when its length could not be read at all, which leaves the ends
/// free rather than pinning them to a guess (missing media must never silently
/// crop a layer).
BarBounds barBounds({
  required int startOffsetFrame,
  required int? sourceFrames,
  required bool retimed,
}) =>
    retimed || sourceFrames == null
        ? BarBounds.free
        : BarBounds(
            minIn: startOffsetFrame,
            maxOut: startOffsetFrame + sourceFrames,
          );

/// How far a grab of [grab] may actually travel when the gesture has moved
/// [delta] frames: inside the layer's source, and never far enough to turn the
/// bar inside out — a bar always keeps at least one frame.
///
/// A **move** is never clamped. Moving carries the start offset with the bar,
/// so a layer that sits inside its source stays inside it however far it
/// travels; only the two trims can run out of source.
///
/// A bound never drags an edge that is *already* outside it — a layer whose
/// Retime was switched off after being stretched keeps the length it has, and
/// its ends stay where the user left them until they are dragged back in.
int clampBarDelta({
  required BarGrab grab,
  required int delta,
  required int inFrame,
  required int outFrame,
  required BarBounds bounds,
}) {
  switch (grab) {
    case BarGrab.move:
      return delta;
    case BarGrab.trimIn:
      var want = inFrame + delta;
      final earliest = bounds.minIn;
      if (earliest != null) want = max(want, min(earliest, inFrame));
      return min(want, outFrame - 1) - inFrame;
    case BarGrab.trimOut:
      var want = outFrame + delta;
      final latest = bounds.maxOut;
      if (latest != null) want = min(want, max(latest, outFrame));
      return max(want, inFrame + 1) - outFrame;
  }
}

/// An exact time as a comp frame number, without asking the engine (K-184).
///
/// The same floor `FrameRate::frame_at` takes, in whole integers so a long
/// timeline cannot drift the way a double would: a time `num/den` seconds at
/// `fpsNum/fpsDen` frames a second is `num·fpsNum / (den·fpsDen)`, rounded
/// down — and down for negative times too, which is what a layer starting
/// before the comp needs.
int frameOfTime(BridgeRational time, int fpsNum, int fpsDen) {
  final den = time.den.toInt() * fpsDen;
  if (den <= 0) return 0;
  final scaled = time.num.toInt() * fpsNum;
  final quotient = scaled ~/ den;
  return scaled % den != 0 && scaled < 0 ? quotient - 1 : quotient;
}

/// The corner marks that say a bar has run out of source (K-211): a small
/// triangle in the top-left corner when the head is as early as its media
/// allows, and one in the top-right when the tail is as late. Drawn only on the
/// kinds that have a source to run out of, and never on a retimed layer, whose
/// ends are free.
class BarEndMarksPainter extends CustomPainter {
  final bool atIn;
  final bool atOut;
  final Color colour;

  /// The triangle's legs. Small enough to read as a corner cut on a 22px row
  /// rather than as a badge sitting on the bar.
  static const double leg = 5;

  const BarEndMarksPainter({
    required this.atIn,
    required this.atOut,
    required this.colour,
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (size.width <= 0) return;
    // Never let the two marks meet in the middle of a very short bar: a bar
    // narrower than both legs draws marks scaled to fit it instead.
    final l = min(leg, size.width / 2);
    final paint = Paint()..color = colour;
    if (atIn) {
      canvas.drawPath(
        Path()
          ..moveTo(0, 0)
          ..lineTo(l, 0)
          ..lineTo(0, l)
          ..close(),
        paint,
      );
    }
    if (atOut) {
      canvas.drawPath(
        Path()
          ..moveTo(size.width, 0)
          ..lineTo(size.width - l, 0)
          ..lineTo(size.width, l)
          ..close(),
        paint,
      );
    }
  }

  @override
  bool shouldRepaint(BarEndMarksPainter old) =>
      old.atIn != atIn || old.atOut != atOut || old.colour != colour;
}
/// One layer's bar: drag its middle to move it, its ends to trim.
class Bar extends StatefulWidget {
  final CompositionReference comp;
  final BridgeLayerEntry entry;
  final TimelineAxis axis;
  final bool razor;

  /// Read when the razor is clicked, not captured when the bar is built.
  final int Function() playheadFrame;

  /// A razor click on this bar, at the frame under the pointer (K-220) — the
  /// panel decides what that cuts, because Shift cuts layers this bar knows
  /// nothing about.
  final void Function(int frame) onRazor;

  /// Where a cut at screen x lands, in comp frames — the same function the
  /// blade's line is drawn with, so the two cannot disagree (docs/07 §4.5).
  final double Function(double x) razorFrameAt;

  /// Clicking (or grabbing) the bar selects its layer.
  final VoidCallback onSelect;

  /// Double-clicking a Sequence layer's bar opens its view, the same as
  /// double-clicking its name (K-248): the clips are what you came for, and
  /// the bar is where you were already looking.
  final VoidCallback? onOpenSequence;
  final VoidCallback onChanged;

  /// Where the live preview is published, for the waveform lane to follow.
  final ValueNotifier<BarDragPreview?> dragPreview;

  /// How far this layer's ends may be dragged (K-211). [BarBounds.free] for
  /// every kind that has no source to run out of.
  final BarBounds bounds;

  /// Whether this layer is in the selection. The bar is the only mark a
  /// selected layer has on the lane side, and with several chosen at once
  /// (K-217) the outline's lit rows are off the side of the panel.
  final bool selected;

  /// Every key on the layer, drawn on its row at half scale while it is shut
  /// (§12A.1) — a summary, not a target: they are not draggable here, because
  /// several properties keyed on one frame are several keys under one diamond.
  /// Twirl the layer open and each property's lane draws its own.
  final List<BridgeKeyframe> summaryKeys;

  /// The comp's rate, to place [summaryKeys] on the frame axis.
  final double fps;

  /// Everything the bar's ends can land on (docs/07 §4.5) — the panel's one
  /// shared list, the same one the lane keys reach for.
  final List<SnapTarget> snapTargets;

  /// Whether the magnet is on; `Ctrl` held suspends it for the moment.
  final bool magnet;

  /// Whether the layer's name is written along the bar — Settings ▸ Interface
  /// ▸ Panels, **off by default** (K-514). Handed down rather than read here,
  /// because a bar rebuilds on every hover and a build has no business
  /// reaching for a settings object (K-184's spirit).
  final bool showName;

  /// What a move drag on a **selected** bar carries (K-720): the unlocked
  /// selection and its comp-zero wall. A callback rather than a value because
  /// it is read once, at drag start, from a handler — the selection at the
  /// moment the hand closes, not the one the panel last built with.
  final SelectionMove Function()? selectionMove;

  const Bar({
    super.key,
    required this.comp,
    required this.entry,
    required this.axis,
    required this.razor,
    required this.selected,
    required this.playheadFrame,
    required this.onRazor,
    required this.razorFrameAt,
    required this.onSelect,
    this.onOpenSequence,
    required this.onChanged,
    required this.dragPreview,
    required this.bounds,
    this.summaryKeys = const [],
    required this.fps,
    this.snapTargets = const [],
    this.magnet = true,
    this.showName = false,
    this.selectionMove,
  });

  @override
  State<Bar> createState() => _BarState();
}

class _BarState extends State<Bar> {
  /// Spots a double-click without putting a recogniser in the razor’s way.
  final DoubleTap _barTaps = DoubleTap();

  /// Frames the gesture has moved so far, held here rather than committed.
  ///
  /// A bar drag has no cheap preview to show — moving a layer in time changes
  /// what every frame contains — so the bar moves in Dart and the document
  /// learns about it once, on release.
  int _delta = 0;

  /// Pixels the gesture has moved so far. The frame delta is always derived
  /// from this running total: rounding each pointer event's own delta to
  /// frames and summing those threw the sub-frame remainders away, so a slow
  /// drag moved less than the pointer and a fast one more — which reads as
  /// mouse acceleration.
  double _deltaPx = 0;
  BarGrab? _grab;

  /// `Escape` while the button is down puts the bar back and writes nothing
  /// (P3, §4.1). The gesture stages in Dart and commits one `set_span` on
  /// release, so abandoning it is simply not making that call.
  final DragEscape _escape = DragEscape();

  /// The selection travelling with this bar's move drag (K-720), read once at
  /// drag start; null for a trim or a single-layer move.
  SelectionMove? _moving;

  /// [_moving]'s ids as the preview publishes them, built once at drag start
  /// rather than sixty times a second in [_publishPreview].
  Set<String>? _movingIds;

  /// How far a selection-mate's move drag says **this** bar travels (K-720):
  /// non-zero only while another selected bar is being dragged and this layer
  /// rides along. Fed by the shared preview notifier, so a drag repaints the
  /// bars it carries and nothing else — the panel never hears a pointer move.
  int _followShift = 0;

  void _followSelectionDrag() {
    final p = widget.dragPreview.value;
    final id = widget.entry.layer.internallayerId.toString();
    final shift =
        p != null && p.layerId != id && p.layers.contains(id) ? p.offsetShift : 0;
    if (shift != _followShift && mounted) {
      setState(() => _followShift = shift);
    }
  }

  @override
  void initState() {
    super.initState();
    widget.dragPreview.addListener(_followSelectionDrag);
  }

  @override
  void didUpdateWidget(Bar old) {
    super.didUpdateWidget(old);
    if (old.dragPreview != widget.dragPreview) {
      old.dragPreview.removeListener(_followSelectionDrag);
      widget.dragPreview.addListener(_followSelectionDrag);
    }
  }

  @override
  void dispose() {
    widget.dragPreview.removeListener(_followSelectionDrag);
    _escape.dispose();
    super.dispose();
  }

  /// Put the bar back where the drag found it, preview and all.
  void _abandon() {
    if (!mounted) return;
    setState(() {
      _delta = 0;
      _deltaPx = 0;
      _grab = null;
      _moving = null;
      _movingIds = null;
      _caught = null;
      widget.dragPreview.value = null;
    });
  }

  /// Where the pointer went DOWN, deciding edge-trim versus move. Down, not
  /// drag-start: a drag's start position is where the slop was exceeded,
  /// which read a fast edge grab as a grab of the middle.
  double _downDx = 0;

  /// The last press landed on an already-selected bar and left the selection
  /// standing for a possible selection drag (K-720); the tap, if the gesture
  /// turns out to be one, owes the select the down withheld.
  bool _keptSelection = false;

  /// The pointer is over this bar's body (§4.1, polish 26). It lifts the
  /// leading edge and nothing else — no size change, no second outline — and
  /// leaves the moment the pointer does (P1).
  bool _hover = false;

  /// What the drag in flight last landed on, so the capture can be drawn — the
  /// same hairline a lane key's drag draws (docs/07 §4.5).
  SnapTarget? _caught;

  /// How far the bar has travelled, in whole frames, with the magnet applied
  /// (§4.1: **both ends are sources**, nearest capture wins).
  ///
  /// The pointer's own travel is kept in [_deltaPx] and the snap taken from it
  /// afresh on every move, never from the snapped answer — otherwise a caught
  /// end could not be pulled off its target again.
  int _snappedDelta(int inFrame, int outFrame) {
    // A *travel*, not a place: the axis's end padding must not be taken off it.
    final raw = widget.axis.framesOfPx(_deltaPx);
    final perFrame = widget.axis.perFrame;
    final magnet = widget.magnet &&
        !snapSuspended(
            controlPressed: HardwareKeyboard.instance.isControlPressed);
    if (!magnet || perFrame <= 0) {
      _caught = null;
      return raw.round();
    }
    // Whichever ends this grab moves. A trim moves one; a move moves both, and
    // then the *nearer* capture is the one that takes the drag.
    final sources = switch (_grab ?? BarGrab.move) {
      BarGrab.move => [inFrame.toDouble(), outFrame.toDouble()],
      BarGrab.trimIn => [inFrame.toDouble()],
      BarGrab.trimOut => [outFrame.toDouble()],
    };
    // The bar's own ends are dropped from its targets: a target sitting where
    // a source already is is a magnet at zero travel, which pins the bar where
    // it started. Every kind at once, because what makes it useless is the
    // frame, not what is standing on it.
    final targets = widget.snapTargets
        .where((s) => s.frame != inFrame && s.frame != outFrame);
    var best = raw.round();
    var bestPx = double.infinity;
    SnapTarget? caught;
    for (final source in sources) {
      final snapped = snapFrame(
        frame: source + raw,
        targets: targets,
        perFrame: perFrame,
        magnet: true,
      );
      final on = snapped.caught;
      if (on == null) continue;
      final px = ((on.frame - (source + raw)) * perFrame).abs();
      if (px >= bestPx) continue;
      bestPx = px;
      caught = on;
      best = (on.frame - source).round();
    }
    _caught = caught;
    return best;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final round = t.shape == ThemeShape.round;
    // ZERO bridge calls (K-184): the span already mapped to comp frames, the
    // kind, and the clip split positions all ride in on the read model.
    final info = widget.entry.info;
    final inFrame = info.inFrame;
    final outFrame = info.outFrame;

    // A locked layer's bar is a fact, not a handle: no move, no trim, no cut
    // — clicking it still selects, so the lock switch stays reachable.
    final held = info.switches.locked;

    final (drawIn, drawOut) = switch (_grab) {
      BarGrab.move => (inFrame + _delta, outFrame + _delta),
      BarGrab.trimIn => (inFrame + _delta, outFrame),
      BarGrab.trimOut => (inFrame, outFrame + _delta),
      // Not necessarily at rest: while a selection-mate's bar is being moved,
      // this bar rides along by the same frames (K-720).
      null => (inFrame + _followShift, outFrame + _followShift),
    };

    final left = widget.axis.xOf(drawIn);
    final width = (widget.axis.xOf(drawOut) - left).clamp(2.0, 1e6);

    // The source's reach travels with a move: sliding a layer along the
    // timeline carries its start offset, so the media it can show moves with
    // it. Without this the marks and the ghost stayed behind while the bar
    // went, and a bar at its limit looked as though it had left the limit.
    // A bar riding a selection-mate's drag is being moved too (K-720).
    final shift = _grab == BarGrab.move ? _delta : _followShift;
    final minIn =
        widget.bounds.minIn == null ? null : widget.bounds.minIn! + shift;
    final maxOut =
        widget.bounds.maxOut == null ? null : widget.bounds.maxOut! + shift;
    // Where the untrimmed source would reach (K-212): drawn behind the bar, so
    // what shows past each end is exactly the material trimmed away. Only when
    // there is something to show — a bar filling its source draws no ghost.
    final ghost = (minIn != null && maxOut != null) &&
            (drawIn > minIn || drawOut < maxOut)
        ? (widget.axis.xOf(minIn), widget.axis.xOf(maxOut))
        : null;

    // **16 inside the row, whatever the row measures** (§12A.6's table, K-451,
    // K-454): the bar is one of the few heights the table gives the same under
    // both densities, so what changes with the setting is the lane ground above
    // and below it — three pixels under Compact, three and a half under
    // Regular. That ground is the point: a stack of bars reads as a stack
    // rather than as one continuous slab, and the row seams the lane overlay
    // draws (K-190) fall on it instead of through a bar's edge. The bar used to
    // fill the row's whole height.
    return SizedBox(
      height: t.density.laneRow,
      // **Both children are keyed.** The ghost comes and goes as the bar is
      // trimmed, and without keys the children were matched by position: the
      // ghost appearing took the bar's slot, so the bar's element — and with it
      // the gesture recogniser holding the drag — was rebuilt from scratch
      // mid-gesture. The bar moved by the first update's frames and then went
      // dead, which is what "dragging a footage edge only moves one frame"
      // was. Keys keep each child matched to its own element however many
      // there are.
      child: Stack(
        children: [
          // What the drag landed on, marked while it holds it (docs/07 §4.5) —
          // the same hairline the lane keys draw, because it is the same
          // service. Behind the bar, so the bar it caught is still legible.
          if (_caught != null)
            Positioned(
              key: ValueKey<String>(
                  'tl-bar-snap-caught-${widget.entry.layer.internallayerId}'),
              left: widget.axis.xOf(_caught!.frame) - 0.5,
              top: 0,
              bottom: 0,
              width: 1,
              child: IgnorePointer(child: ColoredBox(color: t.accent)),
            ),
          if (ghost != null)
            Positioned(
              key: ValueKey<String>(
                  'tl-bar-ghost-${widget.entry.layer.internallayerId}'),
              left: ghost.$1,
              width: (ghost.$2 - ghost.$1).clamp(1.0, 1e6),
              top: clipBarInsetFor(t.density),
              height: clipBarHeight,
              child: IgnorePointer(
                child: Container(
                  decoration: BoxDecoration(
                    // A hairline and nothing inside it (§12A.1): the outline
                    // says how far this same clip *could* still be pulled, and
                    // a fill would read as a second, dimmer object sitting
                    // behind the bar rather than as the bar's own reach.
                    border: Border.all(
                      color: t.labelColour(info.label).withValues(alpha: 0.25),
                      width: 1,
                    ),
                    // Follows the bar's own ends: this *is* the bar, drawn as
                    // far as its source goes, and a rectangle round a capsule
                    // would read as a second object rather than the same one.
                    borderRadius: BorderRadius.circular(
                        round ? t.tokens.controlRadius : sharpClipRadius),
                  ),
                ),
              ),
            ),
          Positioned(
            key: ValueKey<String>(
                'tl-bar-body-${widget.entry.layer.internallayerId}'),
            left: left,
            width: width,
            top: clipBarInsetFor(t.density),
            height: clipBarHeight,
            // Selection on the raw DOWN, outside the gesture arena: the
            // bar's tap otherwise waits for the move/trim drag recognisers
            // to concede before the Effect controls learn the layer.
            child: MouseRegion(
              key: ValueKey<String>(
                  'tl-bar-cursor-${widget.entry.layer.internallayerId}'),
              // The bar's body is a handle, and the cursor says so before the
              // button goes down (P2, §4.1). The two end strips lie over this
              // one and keep their resize arrows; a **locked** bar, and an
              // armed razor, take the plain arrow instead — neither is a grab,
              // and `forbidden` belongs to a refused drop rather than to a
              // surface at rest (P1).
              cursor: held || widget.razor
                  ? SystemMouseCursors.basic
                  : _grab != null
                      ? SystemMouseCursors.grabbing
                      : SystemMouseCursors.grab,
              onEnter: (_) => setState(() => _hover = true),
              onExit: (_) => setState(() => _hover = false),
              child: Listener(
                onPointerDown: (event) {
                  if (event.buttons != kPrimaryButton) return;
                  // A plain press on a bar **already in the selection** leaves
                  // the selection standing (K-720): the gesture may be a move
                  // drag of the whole set, and collapsing on the way down
                  // threw the set away before the drag could carry it. A
                  // press that turns out to be only a click still collapses —
                  // in `onTap` below, which a won drag never reaches — so a
                  // click on a selected bar means what it always has. A
                  // modified press keeps its old manners (Ctrl toggles on the
                  // down), and so does the razor, whose click is a cut.
                  final keys = HardwareKeyboard.instance;
                  _keptSelection = !widget.razor &&
                      widget.selected &&
                      !keys.isControlPressed &&
                      !keys.isMetaPressed &&
                      !keys.isShiftPressed;
                  if (!_keptSelection) widget.onSelect();
                  // A Sequence layer's bar opens its view on a double-click, the
                  // same as its name does (K-248) — counted here rather than
                  // with an `onDoubleTap` below, because a double-tap recogniser
                  // beside the razor's `onTapUp` makes the arena hold every
                  // single tap back, and the razor stops cutting ([DoubleTap]).
                  final open = widget.onOpenSequence;
                  if (open != null && _barTaps.tap()) open();
                },
                child: GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  // Armed razor: a click cuts this layer **where it was clicked**
                  // rather than starting a drag (docs/07 §4.4). At the playhead
                  // is what Cut-at-playhead is for; a razor's whole point is that
                  // the cut lands under the blade. A layer with nothing cuttable
                  // there says so through the engine's calm error, which is
                  // nothing on screen — the cut simply does not happen.
                  onTapUp: widget.razor && !held
                      ? (details) => widget.onRazor(
                            widget
                                .razorFrameAt(left + details.localPosition.dx)
                                .round(),
                          )
                      : null,
                  // Selection usually happened on the down; registering the tap
                  // keeps the click out of any parent recogniser's hands either
                  // way. When the down kept a multi-selection standing (K-720)
                  // and no drag claimed the gesture, the tap is the click it
                  // turned out to be, and selects now — on the way up, exactly
                  // what the down used to do.
                  onTap: widget.razor && !held
                      ? null
                      : () {
                          if (!_keptSelection) return;
                          _keptSelection = false;
                          widget.onSelect();
                        },
                  onHorizontalDragDown: widget.razor || held
                      ? null
                      : (d) => _downDx = d.localPosition.dx,
                  supportedDevices: dragDevices,
                  onHorizontalDragStart: widget.razor || held
                      ? null
                      // No select here: every drag begins with the down, and the
                      // down already selected.
                      : (d) {
                          setState(() {
                            _delta = 0;
                            _deltaPx = 0;
                            _grab = barGrabAt(_downDx, width);
                            // A move that starts on a selected bar carries the
                            // whole unlocked selection (K-720); a real plural
                            // only, and only while this bar is still in it — a
                            // Ctrl press on the way down may have just toggled
                            // it out. Trims stay single-layer whatever is
                            // selected.
                            // ponytail: no multi-trim — one edge over many
                            // bars has no one honest answer yet; build it when
                            // testers reach for it.
                            _moving = null;
                            _movingIds = null;
                            if (_grab == BarGrab.move && widget.selected) {
                              final move = widget.selectionMove?.call();
                              final mine = widget.entry.layer.internallayerId;
                              if (move != null &&
                                  move.layerIds.length > 1 &&
                                  move.layerIds.contains(mine)) {
                                _moving = move;
                                _movingIds = {
                                  for (final id in move.layerIds) id.toString(),
                                };
                              }
                            }
                          });
                          _escape.begin(_abandon);
                        },
                  onHorizontalDragUpdate: widget.razor || held
                      ? null
                      // Nothing moves once `Escape` has taken the drag, though
                      // the pointer carries on: the bar is back where it started
                      // and stays there until the button comes up.
                      : (d) {
                          if (!_escape.running) return;
                          setState(() {
                            _deltaPx += d.delta.dx;
                            // The pointer keeps travelling; the bar does not.
                            // Held against the source's ends (K-211) and against
                            // itself, so a trim can neither run past the media
                            // nor turn the bar inside out — and dragging back
                            // picks the edge up again from where it stuck.
                            _delta = clampBarDelta(
                              grab: _grab ?? BarGrab.move,
                              delta: _snappedDelta(inFrame, outFrame),
                              inFrame: inFrame,
                              outFrame: outFrame,
                              bounds: widget.bounds,
                            );
                            // The selection's wall (K-720): the earliest bar
                            // in the set meeting comp zero stops all of them,
                            // the same clamp the engine's slide applies — so
                            // the preview and the commit agree to the frame.
                            final moving = _moving;
                            if (moving != null) {
                              _delta = max(_delta, -moving.minIn);
                            }
                            _publishPreview();
                          });
                        },
                  onHorizontalDragEnd: widget.razor || held
                      ? null
                      : (_) {
                          if (_escape.end()) _commit(inFrame, outFrame);
                        },
                  onHorizontalDragCancel: widget.razor || held
                      ? null
                      : () {
                          _escape.end();
                          _abandon();
                        },
                  child: Container(
                    key: ValueKey<String>(
                        'tl-bar-fill-${widget.entry.layer.internallayerId}'),
                    decoration: BoxDecoration(
                      // The layer's label colour (K-188): the same chip the
                      // outline swatch shows, so recolouring a layer recolours
                      // its bar — and each kind starts on its own colour.
                      // **Desaturated** under the redesign (§12A.1): the fill is
                      // that colour at [clipFillAlpha] over the lane's ground,
                      // computed from the token rather than picked, so a lane
                      // full of layers reads organised rather than carnival. The
                      // solid leading edge below carries the full colour.
                      // Selected bars brighten that fill rather than growing an
                      // outline: the hue still says which layer this is, and a
                      // lighter bar reads at a glance where a 1px box did not.
                      color: widget.selected
                          ? Color.lerp(t.labelColour(info.label), t.textPrimary,
                                  0.35)!
                              .withValues(alpha: clipFillSelectedAlpha)
                          : t
                              .labelColour(info.label)
                              .withValues(alpha: clipFillAlpha),
                      // Stadium ends under Round (K-394, §12.1) — the control
                      // radius is the sentinel that clamps to half the bar's own
                      // height. **The bar's HIT rect is unchanged and stays
                      // rectangular**: a BoxDecoration's radius paints, it does
                      // not hit-test, so [barGrabAt] still reads dx across the
                      // full width and the trim zones keep exactly the grab area
                      // they had. That is deliberate — a curved end would take
                      // pixels off the corner of a target already only 8 px wide.
                      borderRadius: BorderRadius.circular(
                          round ? t.tokens.controlRadius : sharpClipRadius),
                    ),
                    child: Stack(
                      children: [
                        // The leading edge (§12A.1): 2px of the full colour at
                        // the bar's start, so a desaturated fill still lands with
                        // a snap and a row of bars reads as a row of beginnings.
                        Positioned(
                          key: ValueKey<String>(
                              'tl-bar-edge-${widget.entry.layer.internallayerId}'),
                          left: 0,
                          top: 0,
                          bottom: 0,
                          width: clipEdgeWidth,
                          child: IgnorePointer(
                            // Hover lifts it a step toward `text_primary`
                            // (§4.1): the edge already rests at the label's full
                            // strength, so there is nothing to firm — what a
                            // hovered bar can do is stand one step nearer the
                            // colour selection speaks in, under the pointer and
                            // nowhere else (P1). A selected bar is already
                            // saying more than that through its fill, and does
                            // not say it twice.
                            child: ColoredBox(
                              color: _hover && !widget.selected
                                  ? Color.lerp(t.labelColour(info.label),
                                      t.textPrimary, clipEdgeHoverLift)!
                                  : t.labelColour(info.label),
                            ),
                          ),
                        ),
                        // The layer's name, on its bar (§6.1, §7.1): **Hanken at
                        // 10**, the mockup's own size (K-451), set clear of the
                        // leading edge. It was mono at 11 — but a layer's name is
                        // something the *user* named, and §7.1 sets those in
                        // sentence-case Hanken; the mono row keeps the axis
                        // numbers and units, which are numbers.
                        //
                        // Full `text_primary`, no alpha: the mockup draws the
                        // name opaque. Quieting it only made a name over a pale
                        // label colour harder to read than the bar it sits on.
                        //
                        // **Only when asked for** (K-514): the mockups draw
                        // the name on every bar and so did the editor, and the
                        // owner's ruling from desktop testing is that it reads
                        // as the outline's own column of names said twice.
                        // Off by default, unchanged when on.
                        if (widget.showName)
                          Positioned(
                            left: clipEdgeWidth + 4,
                            right: 2,
                            top: 0,
                            bottom: 0,
                            child: IgnorePointer(
                              child: Align(
                                alignment: Alignment.centerLeft,
                                child: Text(
                                  info.name,
                                  key: ValueKey<String>(
                                      'tl-bar-name-${widget.entry.layer.internallayerId}'),
                                  style: t.small.copyWith(color: t.textPrimary),
                                  maxLines: 1,
                                  overflow: TextOverflow.clip,
                                  softWrap: false,
                                ),
                              ),
                            ),
                          ),
                        // A Sequence layer's bar stays a plain bar: the clips and
                        // their edit points are the sequence view's to draw, and
                        // split lines up here only said the same thing twice
                        // (K-248). What the bar does show is where its clips are
                        // *not* — the gaps, faint, the way a trimmed footage
                        // layer shows the source it is not using (K-212).
                        if (info.kind == BridgeLayerKind.sequence)
                          Positioned.fill(
                            child: IgnorePointer(
                              child: CustomPaint(
                                painter: SequenceGapsPainter(
                                  clips: info.clips,
                                  axis: widget.axis,
                                  left: left,
                                  ink: t.surface0,
                                ),
                              ),
                            ),
                          ),
                        // The two trim zones say so under the pointer: a bar
                        // whose ends can be taken hold of should not have to be
                        // discovered by trial. Inside the gesture detector, not
                        // over it, so hovering never costs the drag its events.
                        if (!held && !widget.razor) ...[
                          _trimCursor(width, left: true),
                          _trimCursor(width, left: false),
                        ],
                        // The corner marks: this bar is as long as its source
                        // allows in that direction (K-211).
                        Positioned.fill(
                          child: IgnorePointer(
                            child: CustomPaint(
                              key: ValueKey<String>(
                                  'tl-bar-ends-${widget.entry.layer.internallayerId}'),
                              painter: BarEndMarksPainter(
                                atIn: minIn != null && drawIn <= minIn,
                                atOut: maxOut != null && drawOut >= maxOut,
                                // The same ink the clip splits use, so the bar
                                // keeps one vocabulary of marks.
                                colour: t.surface0,
                              ),
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          ),
          // What is keyed inside a shut layer, at half scale (§12A.1) — so
          // the stack says where the animation is without every layer having
          // to be twirled open. They travel with a move, because they belong
          // to the layer.
          if (widget.summaryKeys.isNotEmpty)
            Positioned.fill(
              key: ValueKey<String>(
                  'tl-bar-keys-${widget.entry.layer.internallayerId}'),
              child: IgnorePointer(
                child: CustomPaint(
                  painter: LaneKeysPainter(
                    frames: [
                      for (final k in widget.summaryKeys)
                        laneKeyFrame(k, widget.fps) + shift,
                    ],
                    selected: const {},
                    axis: widget.axis,
                    colour: t.animated,
                    chosen: t.textPrimary,
                    half: _summaryKeyHalf,
                  ),
                ),
              ),
            ),
          // The layer's own markers (K-254), over the bar so they take the
          // pointer ahead of it — a flag is a much smaller target than a bar,
          // and a right-click meant for one must not open the bar's menu.
          // They travel with a move, because they are part of the layer.
          for (final m in info.markers)
            Positioned(
              left: widget.axis.xOf(m.frame.toInt() + shift) -
                  MarkerFlag.width / 2,
              bottom: 0,
              child: MouseRegion(
                cursor: SystemMouseCursors.click,
                child: GestureDetector(
                  key: ValueKey<String>('tl-layer-marker-${m.marker.id}'),
                  behavior: HitTestBehavior.opaque,
                  onSecondaryTapUp: (d) =>
                      _markerMenu(context, m.marker, d.globalPosition),
                  // A left click on a flag is a click on its layer, which is
                  // what the bar under it would have done.
                  onTap: widget.onSelect,
                  child: MarkerFlag(
                    label: m.marker.label,
                    fill: t.marker,
                    pill: t.surface4,
                    text: markerLabelStyle(t),
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }

  /// The right-click menu on a marker sitting on a layer's bar — the shared
  /// marker menu, with Delete all on it.
  ///
  /// Deleting here touches **this layer's** list and nothing else. A layer's
  /// markers are its own copy of whatever composition was dropped in, so a
  /// delete cannot reach into that comp — or into the other places it is used
  /// (K-254).
  void _markerMenu(BuildContext context, BridgeMarker marker, Offset at) {
    showMarkerMenuFrb(
      context: context,
      position: at,
      marker: marker,
      markers: () => [for (final m in widget.entry.info.markers) m.marker],
      write: (markers) {
        widget.entry.layer.setMarkers(markers: markers);
        widget.onChanged();
      },
      deleteAll: true,
      keyPrefix: 'tl-layer-marker',
    );
  }

  /// One end's hover strip: the pointer becomes the horizontal resize arrow
  /// over exactly the width [barGrabAt] treats as that end.
  Widget _trimCursor(double width, {required bool left}) {
    final edge = min(_trimGrab, width / 3);
    return Positioned(
      left: left ? 0 : null,
      right: left ? null : 0,
      top: 0,
      bottom: 0,
      width: edge,
      child: const MouseRegion(
        cursor: SystemMouseCursors.resizeLeftRight,
        child: SizedBox.expand(),
      ),
    );
  }

  /// Publish where the gesture has the bar right now — and, on a selection
  /// move (K-720), where it has every bar riding along — for the waveform
  /// lanes, the key lanes and the mates' own bars to follow.
  void _publishPreview() {
    final grab = _grab;
    if (grab == null) return;
    widget.dragPreview.value = barDragPreview(
        widget.entry.layer.internallayerId.toString(), grab, _delta,
        moving: _movingIds);
  }

  /// One write for the whole gesture, so a move that shifted the in point
  /// and the start offset together is a single undo step — and a move that
  /// carried the whole selection is **still one** (K-720): a single
  /// `slide_layers` batch, not one `set_span` per layer.
  void _commit(int inFrame, int outFrame) {
    final grab = _grab;
    final moving = _moving;
    // Clamped once more on the way out: a source length that arrived from its
    // probe part-way through the gesture only reaches the bar on the next
    // build, and what is committed must obey the bounds in force at release.
    var delta = grab == null
        ? 0
        : clampBarDelta(
            grab: grab,
            delta: _delta,
            inFrame: inFrame,
            outFrame: outFrame,
            bounds: widget.bounds,
          );
    if (moving != null) delta = max(delta, -moving.minIn);
    setState(() {
      _delta = 0;
      _deltaPx = 0;
      _grab = null;
      _moving = null;
      _movingIds = null;
      _caught = null;
    });
    widget.dragPreview.value = null;
    if (grab == null || delta == 0) return;

    if (grab == BarGrab.move && moving != null) {
      widget.comp.slideLayers(layerIds: moving.layerIds, delta: delta);
      widget.onChanged();
      return;
    }

    final span = widget.entry.info.span;
    var newIn = inFrame;
    var newOut = outFrame;
    var offsetShift = 0;
    switch (grab) {
      case BarGrab.move:
        newIn += delta;
        newOut += delta;
        // Moving carries the content with the bar, so time 0 travels too.
        offsetShift = delta;
      case BarGrab.trimIn:
        newIn += delta;
      case BarGrab.trimOut:
        newOut += delta;
    }
    // A bar cannot be trimmed past itself; the op refuses it, and refusing here
    // first means the gesture simply stops rather than raising.
    if (newOut <= newIn) return;

    widget.entry.layer.setSpan(
      span: BridgeSpan(
        inPoint: widget.comp.timeOfFrame(frame: newIn),
        outPoint: widget.comp.timeOfFrame(frame: newOut),
        startOffset: offsetShift == 0
            ? span.startOffset
            : widget.comp.timeOfFrame(
                frame: widget.comp.frameAtTime(time: span.startOffset) +
                    offsetShift,
              ),
      ),
    );
    widget.onChanged();
  }
}

/// Which part of a bar a drag grabbed: its middle, or one of its two ends.
enum BarGrab { move, trimIn, trimOut }
