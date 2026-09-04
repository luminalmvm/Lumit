// One keyed property's lane in the Timeline, and the row dividers drawn behind
// the lanes.
//
// Split out of timeline_panel_frb.dart.

import 'package:flutter/foundation.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import '../widgets/drag_escape.dart';
import 'key_block.dart';
import 'timeline_extras_frb.dart';
import 'layer_fold_frb.dart';
import 'timeline_snap.dart';
import 'timeline_metrics_frb.dart';
import 'timeline_bar_frb.dart';
import 'timeline_outline_frb.dart';
import 'timeline_key_block_frb.dart';

/// One keyed property's lane: its keyframes as diamonds, each draggable in
/// time.
///
/// With the magnet on, a drag lands on whole frames; with it off the key may
/// sit *between* frames (docs/07 §4.5) — the times are exact rationals either
/// way. The gesture holds its offset in Dart and commits once on release, so
/// a drag is one undo step; a move onto a neighbour is refused and the key
/// simply stays where it was.
class KeyLane extends StatefulWidget {
  final BridgeLayerEntry entry;
  final LayerFoldRow row;
  final String rowId;
  final List<BridgeKeyframe> keys;
  final TimelineAxis axis;
  final double fps;
  final int fpsNum;
  final int fpsDen;
  final bool magnet;

  /// How far this layer's bar has been moved by a drag in flight, in frames
  /// (§6.26) — [keyShiftOf]. Zero except while its own bar is being moved.
  final int barShift;

  /// Everything on the Timeline this lane's keys may land on (docs/07 §4.5),
  /// gathered once for the panel and handed down — the list is the same for
  /// every lane, so building it per lane would be the same work many times.
  /// This lane's own keys are already left out of it.
  final List<SnapTarget> snapTargets;
  final Set<String> selectedKeys;

  /// The block stretch in flight. A key this gesture holds draws where
  /// the stretch puts it, so the diamonds travel with the box rather than
  /// waiting for the release — the same live reading a bar drag gives.
  final ValueNotifier<KeyStretch?> stretch;

  /// Click a diamond to select it — the second way into the key selection the
  /// F9 family and the easing buttons act on, beside the marquee. Additive
  /// (Shift, Ctrl) toggles one in or out of the catch.
  final void Function(int index, bool additive) onSelectKey;

  /// Right-click on one of this lane's diamonds, at the pointer in global
  /// coordinates.
  final void Function(int index, Offset position) onKeyMenu;

  /// The release of a key drag: write the whole selection where the gesture
  /// has carried it (6.24). The area does the writing, because the keys are on
  /// rows this lane does not have.
  final ValueChanged<KeyStretch> onMoveKeys;
  final VoidCallback onChanged;

  const KeyLane({
    super.key,
    required this.entry,
    required this.row,
    required this.rowId,
    required this.keys,
    required this.axis,
    required this.fps,
    required this.fpsNum,
    required this.fpsDen,
    required this.magnet,
    required this.barShift,
    required this.snapTargets,
    required this.selectedKeys,
    required this.stretch,
    required this.onSelectKey,
    required this.onKeyMenu,
    required this.onMoveKeys,
    required this.onChanged,
  });

  @override
  State<KeyLane> createState() => _KeyLaneState();
}

/// Past this many keys a lane stops building a widget per key and takes the
/// hit-strip below instead. An imported After Effects camera arrives with a
/// baked key per frame — thousands per property — and a `Positioned` +
/// `MouseRegion` + `GestureDetector` per key made every rebuild, layout and
/// hit-test of the panel pay for all of them (the ~10 fps "camera twirled
/// open" report). Hand-keyed lanes stay under this and keep the per-key
/// widgets, whose keys the tests drive by name.
const int keyLaneSlotBudget = 64;

class _KeyLaneState extends State<KeyLane> {
  int? _dragging;

  /// Where each key draws, in lane pixels, for the frame being built — what
  /// the dense strip resolves a pointer against. In key order, which is
  /// x-sorted except while a stretch carries a subset; a drag in flight owns
  /// the pointer, so nothing resolves against the moved marks anyway.
  List<double> _xs = const [];

  /// The dense strip's hovered key, feeding the painter directly
  /// ([LaneKeysPainter.hoverOf]) so a hover crossing thousands of marks
  /// repaints one lane and rebuilds nothing.
  final ValueNotifier<int?> _hoverIx = ValueNotifier<int?>(null);

  /// The keys this drag is carrying, captured at its start (6.24) — the whole
  /// lane selection, which is spread across rows this lane cannot see.
  Set<String> _held = const {};

  /// Whether a modifier was down when the gesture began, so a release that
  /// turns out to have been a click toggles rather than replaces.
  bool _additive = false;

  /// Whether the start of the gesture already took the key into the selection.
  /// If it did, a release that turns out to have been a click must not act
  /// again: `Ctrl` would toggle straight back out the key the press just added.
  bool _pickedAtStart = false;

  /// Which of this lane's keys the pointer is over (§4.2, polish 26). The mark
  /// brightens halfway to `text_primary` — the pre-selection hint, saying "this
  /// is the one a click would take" — and says nothing else: the time and the
  /// value appear only once a drag starts (P1).
  int? _hovered;

  /// Pixels the gesture has moved. The frame offset is always derived from
  /// this running total rather than summed per event, for the same reason the
  /// bar drag does it: per-event rounding reads as mouse acceleration.
  double _deltaPx = 0;

  /// What the drag in flight last landed on, so the capture can be drawn. The
  /// spec requires the target to be indicated at the moment it takes the drag —
  /// without it a key that jumps reads as a fault rather than a service.
  SnapTarget? _caught;

  /// `Escape` while the button is down puts the key back and writes nothing
  /// (P3). The drag was staged in Dart and had written nothing yet, so the way
  /// out costs no undo step — it simply never becomes one.
  final DragEscape _escape = DragEscape();

  @override
  void dispose() {
    _escape.dispose();
    _hoverIx.dispose();
    super.dispose();
  }

  /// The key nearest [dx], within the twelve-pixel grab the per-key slots
  /// give, or null on open ground. Binary search over [_xs].
  int? _nearestKey(double dx) {
    if (_xs.isEmpty) return null;
    var lo = 0;
    var hi = _xs.length;
    while (lo < hi) {
      final mid = (lo + hi) >> 1;
      if (_xs[mid] < dx) {
        lo = mid + 1;
      } else {
        hi = mid;
      }
    }
    int? best;
    var span = 6.0 + 1e-9;
    // The neighbour on each side of the insertion point; on a tie the later
    // key wins, which is what the stacked per-key slots answered too.
    for (final i in [lo - 1, lo]) {
      if (i < 0 || i >= _xs.length) continue;
      final d = (dx - _xs[i]).abs();
      if (d <= span) {
        span = d;
        best = i;
      }
    }
    return best;
  }

  /// A key drag begins — the strip and the per-key detectors share this.
  void _beginKeyDrag(int i) {
    final keyboard = HardwareKeyboard.instance;
    // Read at the gesture's **start**, because the modifier decides what the
    // gesture means when it begins.
    _additive = keyboard.isShiftPressed ||
        keyboard.isControlPressed ||
        keyboard.isMetaPressed;
    // **A key already in the catch keeps the catch** (6.24) — the rule the
    // graph's own key drag follows, and the rule the key menu already
    // followed. A key *outside* the catch takes it, so a drag always carries
    // something that includes the key in hand.
    _pickedAtStart = !widget.selectedKeys.contains('${widget.rowId}#$i');
    if (_pickedAtStart) widget.onSelectKey(i, _additive);
    setState(() {
      _dragging = i;
      _deltaPx = 0;
      // Captured now so the set cannot change underneath the drag, exactly as
      // the block stretch captures its own.
      _held = {...widget.selectedKeys};
    });
    _escape.begin(_abandon);
  }

  /// Where key [i] draws — its own time, plus whichever gesture has hold of
  /// it: this lane's own drag, or the block stretch running across every lane.
  double _frameOf(int i) {
    // The bar's own travel first (§6.26): while its layer is being moved every
    // key on it draws that far along, which is where the release will leave
    // them. Nothing else here can be in flight at the same time — one pointer,
    // one gesture — so the shift simply moves the ground under all three
    // branches below.
    final base =
        laneKeyFrame(widget.keys[i], widget.fps) + widget.barShift.toDouble();
    // A block stretch and a single key's drag cannot both be in flight — the
    // handle and the diamond are two gestures on one pointer — so the stretch
    // is answered first and answered whole.
    final held = widget.stretch.value;
    // **Interpolated, not escaped.** This read `'\${widget.rowId}#\$i'` — a
    // literal dollar sign and a literal `i` — so the set never held a matching
    // id, the test never passed, and every diamond sat still while the box
    // moved over it until the release put them somewhere they had not been
    // seen to travel (§4.3).
    if (held != null && held.keys.contains('${widget.rowId}#$i')) {
      return held.frameOf(
        base,
        whole: widget.magnet &&
            !snapSuspended(
                controlPressed: HardwareKeyboard.instance.isControlPressed),
      );
    }
    return base;
  }

  /// Where the drag has taken the key it holds: the pointer's travel, held
  /// inside the axis and taken to the nearest target (docs/07 §4.5).
  ///
  /// The one key the hand is on decides the travel; [_publish] then applies
  /// that travel to the whole selection, so a run of keys keeps its shape
  /// rather than each of them finding its own target — the rule the graph's
  /// key drag has always followed.
  double _snappedFrame(int i) {
    final base =
        laneKeyFrame(widget.keys[i], widget.fps) + widget.barShift.toDouble();
    final perFrame = widget.axis.perFrame;
    final moved = perFrame <= 0 ? base : base + _deltaPx / perFrame;
    final clamped = moved.clamp(0.0, widget.axis.frames.toDouble());
    final own = {
      for (final k in widget.keys) laneKeyFrame(k, widget.fps),
    };
    final snapped = snapFrame(
      frame: clamped,
      // This lane's own keys are dropped: a key snapping to itself would be
      // pinned where it started, and a neighbour already on the same frame is
      // not a place worth being taken to either.
      targets: widget.snapTargets
          .where((t) => t.kind != SnapKind.keyframe || !own.contains(t.frame)),
      perFrame: perFrame,
      // `Ctrl` held suspends snapping for as long as it is held, which is the
      // way out when the wanted place is exactly where a snap will not allow.
      magnet: widget.magnet &&
          !snapSuspended(
              controlPressed: HardwareKeyboard.instance.isControlPressed),
    );
    _caught = snapped.caught;
    return snapped.frame;
  }

  /// Tell every lane how far the gesture has carried the selection (6.24).
  ///
  /// Broadcast rather than kept here for the reason the block stretch is: the
  /// keys in hand sit on rows in another part of the tree, and a travel only
  /// this lane knew about would move one diamond while the rest of the
  /// selection sat still until the release put them somewhere they had not
  /// been seen to go.
  ///
  /// Nothing to say while the travel is zero, and so nothing published: a
  /// gesture that has not moved leaves every key reading its own time, which is
  /// what keeps a key already sitting between frames from being rounded onto
  /// one by a drag that went nowhere.
  void _publish() {
    final i = _dragging;
    if (i == null) return;
    final base =
        laneKeyFrame(widget.keys[i], widget.fps) + widget.barShift.toDouble();
    final by = _snappedFrame(i) - base;
    widget.stretch.value =
        by == 0 ? null : KeyStretch.shift(keys: _held, by: by);
  }

  /// Put the key back where the drag found it, leaving nothing behind — what
  /// `Escape` does, and what a cancelled gesture does.
  void _abandon() {
    widget.stretch.value = null;
    if (!mounted) return;
    setState(() {
      _dragging = null;
      _deltaPx = 0;
      _caught = null;
    });
  }

  /// The release: every key the gesture held written where it has been seen to
  /// travel, as one undo step (6.24).
  ///
  /// **A gesture that never moved was a click**, and a click on a key selects
  /// exactly that key. The two are told apart here rather than by a second
  /// recogniser because the diamond has only one, deliberately: a tap
  /// recogniser beside the drag would make the drag wait out the slop, and the
  /// pixels it waited through are frames the key would never travel.
  void _commit(int index) {
    final moved = widget.stretch.value;
    final wasClick = _deltaPx == 0;
    widget.stretch.value = null;
    setState(() {
      _dragging = null;
      _deltaPx = 0;
      _caught = null;
    });
    if (wasClick) {
      if (!_pickedAtStart) widget.onSelectKey(index, _additive);
      return;
    }
    // A drag that stayed inside the frame it started on writes nothing — there
    // is no travel to apply, and an undo step for it would undo nothing.
    if (moved != null) widget.onMoveKeys(moved);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Rebuilt while a block stretch runs, so this lane's keys travel with the
    // box. Listening rather than reading once: the stretch is written
    // by a handle in another part of the tree, and a lane that only read it at
    // build time would show the box moving over keys that had not.
    return ValueListenableBuilder<KeyStretch?>(
      valueListenable: widget.stretch,
      builder: (context, _, __) => _lane(context, t),
    );
  }

  Widget _lane(BuildContext context, LumitTheme t) {
    // Worked out once for the build: [_frameOf] is where the snap is decided
    // and where [_caught] is set, so asking it twice per key would answer the
    // same question twice and leave the indicator depending on which of the
    // two calls ran last.
    final frames = [for (var i = 0; i < widget.keys.length; i++) _frameOf(i)];
    _xs = [for (final f in frames) widget.axis.xOf(f)];
    final caught = _caught;
    final dragging = _dragging;
    final dense = widget.keys.length > keyLaneSlotBudget;
    // **Every child of this Stack carries a key**, and the keys stay the same
    // whether or not a snap has been caught.
    //
    // Without them the drag died the moment a snap first took it, which read as
    // "a lane key can only be dragged one frame, and dragging again puts it
    // back". A child appearing part-way down an unkeyed list makes Flutter pair
    // each new child with the *old* child in that slot — the indicator was
    // matched to the first diamond, the first diamond to the second, and so on —
    // so the diamonds' gesture detectors were torn down and rebuilt mid-gesture.
    // A recogniser destroyed while it holds a pointer ends its drag, which
    // committed the two or three pixels travelled so far and left the rest of
    // the gesture doing nothing. Keyed, each child is matched to itself, the
    // detector holding the pointer lives, and the drag runs to the release.
    return Stack(
      children: [
        Positioned.fill(
          key: const ValueKey<String>('tl-lane-diamonds'),
          child: CustomPaint(
            painter: LaneKeysPainter(
              frames: frames,
              selected: {
                for (var i = 0; i < widget.keys.length; i++)
                  if (widget.selectedKeys.contains('${widget.rowId}#$i')) i,
              },
              axis: widget.axis,
              colour: t.animated,
              chosen: t.textPrimary,
              hovered: dense ? null : _hovered,
              hoverOf: dense ? _hoverIx : null,
              // The same size and the same shapes in both modes: a key says
              // its interpolation wherever it is drawn, and says it at the
              // size it is aimed at.
              shapes: [for (final k in widget.keys) keyShapeOf(k)],
            ),
          ),
        ),
        // What the drag landed on, marked while it holds it (docs/07 §4.5:
        // the snapped-to target MUST be indicated at the moment of capture).
        if (caught != null)
          Positioned(
            key: const ValueKey<String>('tl-lane-snap-caught'),
            left: widget.axis.xOf(caught.frame) - 0.5,
            top: 0,
            bottom: 0,
            width: 1,
            child: IgnorePointer(
              child: ColoredBox(color: t.accent),
            ),
          ),
        if (dense)
          // One widget for the whole lane past the slot budget: the strip's
          // render object claims only the pixels a per-key slot would have
          // (twelve around each mark, so the marquee keeps the ground), and
          // the handlers resolve which key from the pointer instead of from a
          // widget per key. Same grabs, same menu, same drag — minus the
          // thousands of widgets an imported camera lane was paying to hold
          // them.
          Positioned.fill(
            key: ValueKey<String>('tl-key-strip-${widget.rowId}'),
            child: MouseRegion(
              cursor: SystemMouseCursors.resizeLeftRight,
              onHover: (d) => _hoverIx.value = _nearestKey(d.localPosition.dx),
              onExit: (_) => _hoverIx.value = null,
              child: GestureDetector(
                behavior: HitTestBehavior.deferToChild,
                supportedDevices: dragDevices,
                onSecondaryTapUp: (d) {
                  final i = _nearestKey(d.localPosition.dx);
                  if (i != null) widget.onKeyMenu(i, d.globalPosition);
                },
                onHorizontalDragStart: (d) {
                  final i = _nearestKey(d.localPosition.dx);
                  if (i != null) _beginKeyDrag(i);
                },
                onHorizontalDragUpdate: (d) {
                  if (!_escape.running || _dragging == null) return;
                  setState(() => _deltaPx += d.delta.dx);
                  _publish();
                },
                onHorizontalDragEnd: (_) {
                  final i = _dragging;
                  if (_escape.end() && i != null) _commit(i);
                },
                onHorizontalDragCancel: () {
                  _escape.end();
                  _abandon();
                },
                child: _KeyStripHit(xs: _xs),
              ),
            ),
          )
        else
          for (var i = 0; i < widget.keys.length; i++)
            Positioned(
              key: ValueKey<String>('tl-key-slot-${widget.rowId}#$i'),
              left: widget.axis.xOf(frames[i]) - 6,
              top: 0,
              width: 12,
              height: t.density.laneRow,
              child: MouseRegion(
                cursor: SystemMouseCursors.resizeLeftRight,
                // The grab slot is the hover target too: what brightens is
                // exactly what a press would take (P5).
                onEnter: (_) => setState(() => _hovered = i),
                onExit: (_) => setState(() {
                  if (_hovered == i) _hovered = null;
                }),
                child: GestureDetector(
                  key: ValueKey<String>('tl-key-${widget.rowId}#$i'),
                  behavior: HitTestBehavior.opaque,
                  // Touching a diamond selects it, and a drag is a touch that
                  // went somewhere — so the drag's own start is where selection
                  // belongs. This recognizer is alone in the arena, which means
                  // it wins on release even when the pointer never moved: one
                  // callback covers the click and the drag, and no second
                  // recognizer competes for the sub-pixel-per-frame movements a
                  // lane drag is made of. Without a per-key selection only the
                  // marquee could fill the lane catch, so easing one key from
                  // the lanes (F9, the bottom bar's buttons) had nothing to act
                  // on and looked like it did nothing.
                  supportedDevices: dragDevices,
                  // The key's own menu — Linear / Easy ease / Hold / Ease… /
                  // Delete key, the graph key's menu, on the mark the lanes
                  // draw.
                  onSecondaryTapUp: (d) =>
                      widget.onKeyMenu(i, d.globalPosition),
                  onHorizontalDragStart: (_) => _beginKeyDrag(i),
                  // Ignored once `Escape` has taken the drag: the pointer keeps
                  // travelling after it, and a key that started following again
                  // would make the way out look like a stutter.
                  onHorizontalDragUpdate: (d) {
                    if (!_escape.running) return;
                    setState(() => _deltaPx += d.delta.dx);
                    _publish();
                  },
                  onHorizontalDragEnd: (_) {
                    if (_escape.end()) _commit(i);
                  },
                  onHorizontalDragCancel: () {
                    _escape.end();
                    _abandon();
                  },
                ),
              ),
            ),
        // The live readout, while the pointer is down: what frame the key has
        // reached and what it holds there (§4.2). Last in the stack so it is
        // over the diamonds, and gone the moment the drag ends — nothing at
        // rest (P1).
        if (dragging != null && dragging < widget.keys.length)
          _dragHint(frames[dragging], widget.keys[dragging].value),
      ],
    );
  }

  /// The `f<frame> · <value>` pill beside the key being dragged.
  ///
  /// The value is the one this lane's own keys carry — a multi-axis row reads
  /// its lead axis, exactly as its diamonds do (`laneKeysOf`) — so the readout
  /// costs nothing but a lookup: a sampled row value would cross the bridge on
  /// every pointer move.
  Widget _dragHint(double frame, double value) {
    final x = widget.axis.xOf(frame);
    // Beside the key, or on its other side where the axis has run out: a
    // readout clipped by the edge is no readout.
    const pill = 72.0;
    final left = x + 8 + pill > widget.axis.width ? x - 8 - pill : x + 8;
    return Positioned(
      key: const ValueKey<String>('tl-key-drag-hint'),
      left: left,
      top: 1,
      child: HintPill(
        text: l10n.timelineKeyDragHint(frame.round(), keysNumberText(value)),
      ),
    );
  }
}

/// The dense lane's hit surface: a paintless box that claims only the pixels
/// within a key slot's reach (six either side of a mark), so the marquee and
/// the ground click keep everything between the keys — exactly the ground the
/// per-key slots left uncovered.
class _KeyStripHit extends LeafRenderObjectWidget {
  /// Key positions in lane pixels, in key order (x-sorted at rest; see
  /// [_KeyLaneState._xs]).
  final List<double> xs;

  const _KeyStripHit({required this.xs});

  @override
  RenderObject createRenderObject(BuildContext context) =>
      _RenderKeyStripHit(xs);

  @override
  void updateRenderObject(
          BuildContext context, covariant _RenderKeyStripHit renderObject) =>
      renderObject.xs = xs;
}

class _RenderKeyStripHit extends RenderBox {
  List<double> xs;

  _RenderKeyStripHit(this.xs);

  @override
  bool get sizedByParent => true;

  @override
  Size computeDryLayout(BoxConstraints constraints) => constraints.biggest;

  @override
  bool hitTest(BoxHitTestResult result, {required Offset position}) {
    if (!size.contains(position)) return false;
    // The nearest key by bisection; within six pixels the strip answers the
    // hit, elsewhere the pointer falls through to the ground below.
    var lo = 0;
    var hi = xs.length;
    while (lo < hi) {
      final mid = (lo + hi) >> 1;
      if (xs[mid] < position.dx) {
        lo = mid + 1;
      } else {
        hi = mid;
      }
    }
    final near = (lo < xs.length && (xs[lo] - position.dx).abs() <= 6.0) ||
        (lo > 0 && (position.dx - xs[lo - 1]).abs() <= 6.0);
    if (!near) return false;
    result.add(BoxHitTestEntry(this, position));
    return true;
  }
}

/// Where the row seams fall, in the painter's own coordinates — one entry per
/// hairline, each already **snapped to a whole pixel**.
///
/// **Why the snapping is the whole point.** Both halves of the Timeline draw
/// their seams with the same painter, but the lane side draws inside the
/// scrolled content (so its seams land on multiples of the row height, whole
/// numbers) while the outline side draws on an overlay pinned to the panel and
/// carries the scroll in [phase] instead. A scroll offset is very rarely a
/// whole number — a wheel flings through physics that lands wherever it lands
/// — so the outline's seams were coming out at fractions of a pixel, and a
/// 1px line drawn at, say, y = 21.8 is painted as two neighbouring rows of
/// grey rather than one row of hairline. That is what "the outline's dividers
/// look 2px and faint where the lanes' look crisp" was, reported from desktop.
///
/// Rounding each seam to the nearest whole pixel first, and *then* offsetting
/// by the half-pixel a 1px stroke needs to sit inside one row of pixels rather
/// than straddle two, gives both halves the identical crisp line.
///
/// [blanks] are stretches to leave unruled, as (top, bottom) pairs: an open
/// sequence view is one table cell, not six rows of one.
List<double> rowSeamOffsets({
  required double step,
  required double height,
  double phase = 0,
  double origin = 0,
  List<(double, double)> blanks = const [],
}) {
  if (step <= 0) return const [];
  final ys = <double>[];
  for (var y = phase + step; y <= height; y += step) {
    if (y < 0) continue;
    // Strictly inside a blank, so the seams that *bound* an open view stay:
    // the row still has a top and a bottom, it simply has no rules through
    // its middle.
    if (blanks.any((b) => y > b.$1 + 0.5 && y < b.$2 - 0.5)) continue;
    // Rounded **where the seam will be seen**, not where it is drawn.
    // [origin] is the painter's own top edge in the panel's pixels — the
    // negated scroll offset for a painter riding the scrolled content, zero
    // for one pinned to the panel. Both halves of the Timeline then put a
    // seam on the same physical pixel for any offset: the lane side used to
    // round in content space, so a fractional scroll offset — which is what
    // clamping to a fractional maxScrollExtent after a panel-height drag
    // leaves behind — slid its lines up to half a pixel off the outline's
    // (owner, 2026-08-31: "the divider lines ... not match up").
    ys.add((y + origin).roundToDouble() - origin - 0.5);
  }
  return ys;
}

/// The lane area's row seams: one hairline per row, the full width of the
/// area.
///
/// Drawn as one overlay rather than given to each row as a border because a
/// decorated box absorbs pointers — a border per row would quietly eat the
/// keyframe marquee under it — and because the bars fill their whole row, so
/// the seam has to land on top of them to be seen at all.
class RowDividerPainter extends CustomPainter {
  final double step;
  final Color colour;

  /// Vertical stretches to leave alone, as (top, bottom) pairs.
  ///
  /// An open sequence view is one table cell, not six rows of one — ruling it
  /// into rows drew lines through the clips and straight across the middle of
  /// the speed envelope, which read as the graph having been chopped up.
  final List<(double, double)> blanks;

  /// How far the first seam sits above the top edge — the outline's overlay
  /// is pinned to the panel rather than to the scrolled rows, so it carries
  /// the scroll offset here instead.
  final double phase;

  /// Where this painter's top edge sits in the panel's own pixels
  /// ([rowSeamOffsets]'s rounding anchor): the negated scroll offset for the
  /// lane side's content-riding overlay, zero for the outline's pinned one.
  final double origin;

  const RowDividerPainter({
    required this.step,
    required this.colour,
    this.phase = 0,
    this.origin = 0,
    this.blanks = const [],
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (step <= 0) return;
    final paint = Paint()
      ..color = colour
      ..strokeWidth = 1;
    for (final y in rowSeamOffsets(
        step: step,
        height: size.height,
        phase: phase,
        origin: origin,
        blanks: blanks)) {
      canvas.drawLine(Offset(0, y), Offset(size.width, y), paint);
    }
  }

  /// The blanks are compared **by value**, not by identity: they are rebuilt
  /// fresh on every build, so an identity test said "changed" every time and
  /// both overlays repainted whatever had actually moved. The list is
  /// one entry per open sequence view, so comparing it is nothing.
  @override
  bool shouldRepaint(RowDividerPainter old) =>
      old.step != step ||
      old.colour != colour ||
      old.phase != phase ||
      old.origin != origin ||
      !listEquals(old.blanks, blanks);

  /// Never absorbs a pointer: a background painter's default would eat the
  /// gestures on the rows below it.
  @override
  bool? hitTest(Offset position) => false;
}
