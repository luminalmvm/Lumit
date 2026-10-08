// Dragging a layer up or down the Timeline's stack (docs/07-UI-SPEC.md §4.7):
// the drag's state, the arithmetic both halves of the table slide by, and the
// widget that moves one layer's block.
//
// In plain terms: while a layer is dragged by its name, the row in hand
// follows the pointer and the rows it passes step out of its way, in the
// outline and in the lanes at once. Let go and it lands in the gap that was
// opened for it. With motion turned down the rows stay where they are and a
// line marks the place instead. Nothing here touches the document. The
// outline row that owns the gesture does that, once, on the drop.
//
// Split out of timeline_metrics_frb.dart, which re-exports it.

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';

import '../theme/motion.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'timeline_extras_frb.dart' show rowSelectionFill;

/// A layer drag in flight: the index lifted, and the index it would land on.
///
/// **Held by the panel and read by both halves of the table**, which is the
/// point. The outline owns the gesture — the name is the stack handle
/// — so when only it knew about the drag, only it could move: the lanes sat
/// still while their layers were being reordered beside them. One value, read
/// by the outline rows and the lane blocks alike, and the two halves slide as
/// one row because they are working from the same number.
class LayerDrag {
  final int from;
  final int to;
  const LayerDrag(this.from, this.to);

  @override
  bool operator ==(Object other) =>
      other is LayerDrag && other.from == from && other.to == to;

  @override
  int get hashCode => Object.hash(from, to);
}

/// How far the block at [index] slides while a drag is in flight, in pixels;
/// positive is down.
///
/// The lifted block travels the whole way to the slot it would take, and every
/// block it passes moves one lift's height the other way — so the stack reads
/// as already reordered before the drop, which is what makes a drop feel
/// decided rather than guessed at. Pure, so the maths both halves depend on is
/// tested without building a Timeline.
double layerDragShift(List<double> heights, LayerDrag? drag, int index) {
  if (drag == null || drag.from == drag.to) return 0;
  if (index < 0 || index >= heights.length) return 0;
  if (drag.from < 0 || drag.from >= heights.length) return 0;
  if (drag.to < 0 || drag.to >= heights.length) return 0;
  if (index == drag.from) {
    var travel = 0.0;
    if (drag.to > drag.from) {
      for (var i = drag.from + 1; i <= drag.to; i++) {
        travel += heights[i];
      }
      return travel;
    }
    for (var i = drag.to; i < drag.from; i++) {
      travel -= heights[i];
    }
    return travel;
  }
  final lifted = heights[drag.from];
  if (drag.to > drag.from) {
    return index > drag.from && index <= drag.to ? -lifted : 0;
  }
  return index >= drag.to && index < drag.from ? lifted : 0;
}

/// Which slot a drag is aiming at, from how far it has travelled.
///
/// [from] is the block lifted, [travel] how far the pointer has moved down the
/// stack since the lift in pixels (negative is up). Returns the index the block
/// would take if dropped now.
///
/// **Measured against the stack as it was when the drag began**, which is the
/// whole point. The rows on screen are slid out of the way while a drag is in
/// flight, so asking "which row is the pointer over?" asks about geometry the
/// drag itself is moving: each answer slides the rows, which changes the next
/// answer, and the block oscillates between two slots without the pointer
/// moving at all. Travel against the original heights cannot do that — it is
/// a function of the pointer alone.
///
/// The threshold is the midpoint of the block being passed, not its edge: an
/// edge means the slot flips the instant a single pixel of overlap appears,
/// which is the other half of the same jitter. Travelling back to where the
/// drag started therefore returns [from] exactly, so a cancelled-by-hand drag
/// leaves the stack alone.
int layerDragTarget(List<double> heights, int from, double travel) {
  if (from < 0 || from >= heights.length) return from;
  var to = from;
  if (travel > 0) {
    var passed = 0.0;
    for (var i = from + 1; i < heights.length; i++) {
      if (travel < passed + heights[i] / 2) break;
      passed += heights[i];
      to = i;
    }
  } else if (travel < 0) {
    var passed = 0.0;
    for (var i = from - 1; i >= 0; i--) {
      if (-travel < passed + heights[i] / 2) break;
      passed += heights[i];
      to = i;
    }
  }
  return to;
}

/// [travel] held to the stack: the block in hand goes no higher than the top
/// row's place and no lower than the bottom row's, however far the pointer
/// runs on past them.
double layerDragReach(List<double> heights, int from, double travel) {
  if (from < 0 || from >= heights.length) return 0;
  var above = 0.0;
  for (var i = 0; i < from; i++) {
    above += heights[i];
  }
  var below = 0.0;
  for (var i = from + 1; i < heights.length; i++) {
    below += heights[i];
  }
  return travel.clamp(-above, below).toDouble();
}

/// Where the line goes that marks a drop's place, in the rows' own pixels
/// from the top of the stack, or null when the drop would move nothing.
///
/// A layer taken down the stack lands under the row it is aimed at and one
/// taken up lands over it, so the line stands on the seam the layer would
/// come to rest against.
double? layerDragMark(List<double> heights, LayerDrag? drag) {
  if (drag == null || drag.from == drag.to) return null;
  if (drag.to < 0 || drag.to >= heights.length) return null;
  var y = 0.0;
  final rows = drag.to > drag.from ? drag.to + 1 : drag.to;
  for (var i = 0; i < rows; i++) {
    y += heights[i];
  }
  return y;
}

/// Where the lifted card stands and how lifted it is, for whatever draws
/// round it.
class LiftedBand {
  /// The card's top and bottom edges, in the rows' own pixels from the top of
  /// the stack.
  final double top;
  final double bottom;

  /// From 0 at rest to 1 in hand.
  final double amount;

  const LiftedBand(this.top, this.bottom, this.amount);

  /// The stretch the row seams leave unruled.
  (double, double) get blank => (top, bottom);

  @override
  bool operator ==(Object other) =>
      other is LiftedBand &&
      other.top == top &&
      other.bottom == bottom &&
      other.amount == amount;

  @override
  int get hashCode => Object.hash(top, bottom, amount);
}

/// What the block in hand did when it was let go.
class LayerDrop {
  /// Where the block stands now it is down, as an index into the rows on
  /// screen: the slot it took, or its own when nothing moved.
  final int index;

  /// How far from that slot it was let go, in pixels down. The block settles
  /// from here.
  final double offset;

  /// Whether the drop reorders the stack. When it does, every other block is
  /// already standing where the new order puts it and must not move again.
  final bool moved;

  const LayerDrop({
    required this.index,
    required this.offset,
    required this.moved,
  });
}

/// The panel's layer drag: the drag in flight, how far the block in hand has
/// been carried, and the last drop.
///
/// Its value is the [LayerDrag], as it always was, so anything that only wants
/// the two indices listens to it as a plain notifier. The carry is a second,
/// separate notifier because it changes on every pointer move and only the one
/// block in hand has anything to do about it.
class LayerDragState extends ValueNotifier<LayerDrag?> {
  LayerDragState() : super(null);

  /// How far the block in hand has been carried from where it was picked up,
  /// in pixels down, held to the stack.
  final ValueNotifier<double> carried = ValueNotifier<double>(0);

  /// The block drawn over its neighbours: the one in hand, then the one that
  /// has just landed. An index into the rows on screen, or null before the
  /// first drag.
  final ValueNotifier<int?> raised = ValueNotifier<int?>(null);

  /// Where the block in hand is drawn, or null when nothing is lifted.
  ///
  /// The row seams are ruled by an overlay above every block, so a block in
  /// hand cannot paint over them. They read this instead and leave the
  /// stretch unruled, which is what puts the lifted layer on top of the lines.
  /// The gutter between the two halves reads it too, to draw the stretch of
  /// card that joins them. Written by the block itself as it moves, so it
  /// follows the settle.
  final ValueNotifier<LiftedBand?> band = ValueNotifier<LiftedBand?>(null);

  /// Where the line marking the drop's place goes ([layerDragMark]), or null.
  /// Only ever set by a drag that is not carried.
  final ValueNotifier<double?> mark = ValueNotifier<double?>(null);

  /// Whether the drag in flight carries its block: the block rides the
  /// pointer and its neighbours step aside. When it does not, every block
  /// stays where it is and [mark] says where a drop would go. Decided at the
  /// lift, from the user's motion setting (`Motion.carries`).
  bool get carries => _carries;
  bool _carries = true;

  bool _disposed = false;

  /// Take the band away, for a block that is leaving the tree while it holds
  /// it. Safe to call after the panel has gone.
  void clearBand() {
    if (!_disposed) band.value = null;
  }

  /// The drop that ended the last drag, read by the blocks as the drag clears.
  LayerDrop? get drop => _drop;
  LayerDrop? _drop;

  /// Pick the block at [index] up. With [carries] false nothing is picked up
  /// to look at: the drag is tracked and a line marks where it is aimed.
  void lift(int index, {bool carries = true}) {
    _carries = carries;
    _drop = null;
    band.value = null;
    mark.value = null;
    carried.value = 0;
    if (carries) raised.value = index;
    value = LayerDrag(index, index);
  }

  /// The pointer has travelled [travel] pixels down the stack since the lift.
  void carry(List<double> heights, double travel) {
    final drag = value;
    if (drag == null) return;
    final to = layerDragTarget(heights, drag.from, travel);
    final next = LayerDrag(drag.from, to);
    if (_carries) {
      carried.value = layerDragReach(heights, drag.from, travel);
    } else {
      mark.value = layerDragMark(heights, next);
    }
    if (to != drag.to) value = next;
  }

  /// Let go. Answers the drag that was in flight, for the caller to commit,
  /// or null when there was none. A [cancelled] drag goes back where it began.
  LayerDrag? release(List<double> heights, {bool cancelled = false}) {
    final drag = value;
    if (drag == null) return null;
    final landed = cancelled ? LayerDrag(drag.from, drag.from) : drag;
    if (!_carries) {
      // Nothing was moved, so nothing has to land.
      mark.value = null;
      value = null;
      return landed;
    }
    _drop = LayerDrop(
      index: landed.to,
      // Where it is on screen, less where its new slot is: both are measured
      // from the row it was lifted from.
      offset: carried.value - layerDragShift(heights, landed, landed.from),
      moved: landed.from != landed.to,
    );
    raised.value = landed.to;
    value = null;
    return landed;
  }

  @override
  void dispose() {
    _disposed = true;
    carried.dispose();
    raised.dispose();
    band.dispose();
    mark.dispose();
    super.dispose();
  }
}

/// How much smaller than its slot a lifted block is drawn, on each side, in
/// pixels. Small enough that an in-row picker still fits the tightest row
/// (16 in Desk's compact 20), and enough for the slot to show round it.
const double layerLiftInsetX = 4;
const double layerLiftInsetY = 1.5;

/// Which stretch of the lifted card a block draws.
///
/// A layer is one row across the whole table, so in hand it is one card. The
/// outline's block draws the card's left end, the lanes' block its right end,
/// and the gutter between them the stretch that joins the two
/// ([LiftBridgePainter]). An end that meets another stretch is drawn square
/// and flush, so no seam shows where they meet.
enum LiftSide {
  /// The whole card, both ends: a stack that stands alone.
  whole,

  /// The left end, running on to the right.
  leading,

  /// The right end, run into from the left.
  trailing,
}

/// The ground a lifted card stands on: the fill a selected row wears under
/// this shape, made opaque so the rows it is carried over do not show through.
///
/// One colour for every stretch of the card. The rows inside it stop drawing
/// their own selection fill while they are in hand ([LayerLift]): each half
/// draws that fill to its own edge, rounded under Lantern, and two fills over
/// one card read as two cards again.
Color liftGround(LumitTheme t) =>
    Color.alphaBlend(rowSelectionFill(t), t.surface2);

/// The colour the card's shadow is cast in.
Color liftShadow(LumitTheme t) => t.floatShadow.first.color;

/// The corner a lifted card's ends are rounded to.
double liftRadius(LumitTheme t) => t.tokens.contentRadius * 2;

/// A number that eases to wherever it is sent, from wherever it has reached.
///
/// Sent somewhere new while it is still moving, it sets off again from where
/// it is, so a row that is passed and passed back never jumps. It makes its
/// controller the first time it is asked to move and not before, so a stack of
/// rows nobody has dragged holds no tickers.
class _Eased {
  _Eased(this._vsync, this._changed);

  final TickerProvider _vsync;
  final VoidCallback _changed;
  AnimationController? _controller;
  double _from = 0;
  double _to = 0;
  Curve _curve = Curves.linear;

  /// Where it is this frame.
  double value = 0;

  /// Put it at [to] with no travel.
  void jump(double to) {
    _controller?.stop();
    _from = _to = value = to;
  }

  /// Send it to [to] along [spec]. A still spec arrives at once.
  void go(double to, MotionSpec spec) {
    if (spec.isStill || to == value) {
      jump(to);
      return;
    }
    if (to == _to && (_controller?.isAnimating ?? false)) return;
    _from = value;
    _to = to;
    _curve = spec.curve;
    final controller = _controller ??= AnimationController(vsync: _vsync)
      ..addListener(_tick);
    controller
      ..duration = spec.duration
      ..forward(from: 0);
  }

  void _tick() {
    value = _from + (_to - _from) * _curve.transform(_controller!.value);
    _changed();
  }

  void dispose() => _controller?.dispose();
}

/// Whether the block a row stands in is in hand, for the row to ask.
///
/// A row in hand leaves its own selection fill to the card under it
/// ([liftGround]). Only the rows that ask are rebuilt, and only when the
/// answer changes: once on the lift and once when the block has landed.
class LayerLift extends InheritedWidget {
  final bool lifted;

  const LayerLift({super.key, required this.lifted, required super.child});

  /// False where nothing above is a layer's block.
  static bool of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<LayerLift>()?.lifted ?? false;

  @override
  bool updateShouldNotify(LayerLift old) => old.lifted != lifted;
}

/// One layer's block, moved by a layer drag.
///
/// The block in hand rides the pointer, drawn over its neighbours on a lifted
/// ground. A block being passed slides one lift's height out of the way. On
/// the drop the block in hand settles into the gap, and since the gap is
/// exactly where the new order puts it, nothing else moves.
///
/// A transform, not a layout change: the rows keep their places, so a drag
/// never reflows the table under itself — and the same widget wraps the block
/// in the outline and the block in the lanes, which is what keeps them
/// together to the pixel.
///
/// How long a slide takes is the theme scope's `motion`, so it follows the
/// shape (docs/15-DESIGN.md §8). At *Minimal* and *None* the drag is not
/// carried at all ([LayerDragState.carries]): no block moves, and a line
/// marks the drop's place instead ([DropMarkPainter]).
class LayerDragSlide extends StatefulWidget {
  final LayerDragState drag;
  final List<double> heights;
  final int index;

  /// Which stretch of the lifted card this block is.
  final LiftSide side;
  final Widget child;

  const LayerDragSlide({
    super.key,
    required this.drag,
    required this.heights,
    required this.index,
    this.side = LiftSide.whole,
    required this.child,
  });

  @override
  State<LayerDragSlide> createState() => _LayerDragSlideState();
}

class _LayerDragSlideState extends State<LayerDragSlide>
    with TickerProviderStateMixin {
  /// How far the block is drawn from its own place, in pixels down.
  late final _Eased _shift = _Eased(this, _repaint);

  /// How lifted the block is drawn, from 0 at rest to 1 in hand.
  late final _Eased _lift = _Eased(this, _repaint);

  /// Whether this is the block in hand, and so listening to the carry.
  bool _inHand = false;

  /// Whether this block is the one the seams are standing clear of.
  bool _bandIsMine = false;

  /// Read in [didChangeDependencies] and kept, because the drag's listeners
  /// run outside a build.
  Motion? _motion;

  void _repaint() {
    _publishBand();
    if (mounted) setState(() {});
  }

  /// Tell the seams where the lifted card is, or that it has come down. Both
  /// halves of the table say the same thing, and the second is ignored.
  void _publishBand() {
    final lift = _lift.value.clamp(0.0, 1.0);
    if (lift <= 0) {
      if (_bandIsMine) {
        _bandIsMine = false;
        widget.drag.band.value = null;
      }
      return;
    }
    final heights = widget.heights;
    if (widget.index >= heights.length) return;
    var top = _shift.value;
    for (var i = 0; i < widget.index; i++) {
      top += heights[i];
    }
    final inset = layerLiftInsetY * lift;
    _bandIsMine = true;
    widget.drag.band.value = LiftedBand(
        top + inset, top + heights[widget.index] - inset, lift);
  }

  @override
  void initState() {
    super.initState();
    widget.drag.addListener(_dragChanged);
    // A block scrolled into view while a drag is already in flight stands
    // where the drag has put it, without travelling there.
    final drag = widget.drag.value;
    if (drag == null || !widget.drag.carries) return;
    if (drag.from == widget.index) {
      _hold(true);
      _shift.jump(widget.drag.carried.value);
      _lift.jump(1);
    } else {
      _shift.jump(layerDragShift(widget.heights, drag, widget.index));
    }
  }

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _motion = ThemeScope.of(context).motion;
  }

  @override
  void didUpdateWidget(LayerDragSlide old) {
    super.didUpdateWidget(old);
    if (old.drag == widget.drag) return;
    old.drag.removeListener(_dragChanged);
    if (_inHand) old.drag.carried.removeListener(_carried);
    _inHand = false;
    widget.drag.addListener(_dragChanged);
    _shift.jump(0);
    _lift.jump(0);
  }

  @override
  void dispose() {
    widget.drag.removeListener(_dragChanged);
    if (_inHand) widget.drag.carried.removeListener(_carried);
    if (_bandIsMine) {
      // After the frame: a notifier may not fire while the tree is being
      // taken down.
      final drag = widget.drag;
      WidgetsBinding.instance.addPostFrameCallback((_) => drag.clearBand());
    }
    _shift.dispose();
    _lift.dispose();
    super.dispose();
  }

  /// Start or stop following the pointer.
  void _hold(bool inHand) {
    if (inHand == _inHand) return;
    _inHand = inHand;
    if (inHand) {
      widget.drag.carried.addListener(_carried);
    } else {
      widget.drag.carried.removeListener(_carried);
    }
  }

  void _carried() {
    _shift.jump(widget.drag.carried.value);
    _repaint();
  }

  void _dragChanged() {
    final motion = _motion;
    if (motion == null) return;
    final drag = widget.drag.value;
    if (drag != null && !widget.drag.carries) {
      // A drag that is only marked: every block stays where it stands.
      _hold(false);
      _shift.jump(0);
      _lift.jump(0);
    } else if (drag != null) {
      if (drag.from == widget.index) {
        _hold(true);
        _shift.jump(widget.drag.carried.value);
        _lift.go(1, motion.lift);
      } else {
        _hold(false);
        _lift.jump(0);
        _shift.go(
            layerDragShift(widget.heights, drag, widget.index), motion.makeWay);
      }
    } else {
      _hold(false);
      final drop = widget.drag.drop;
      if (drop != null && drop.index == widget.index) {
        // The block that landed. After a reorder that is whichever block now
        // stands in the slot, which is not the one that was lifted: the rows
        // are rebuilt in their new order in this same frame. It takes the
        // band over with the slot, and gives it up when it has settled.
        _bandIsMine = true;
        _shift.jump(drop.offset);
        _shift.go(0, motion.settle);
        _lift.jump(1);
        _lift.go(0, motion.settle);
      } else if (drop == null || drop.moved) {
        // Already standing where the new order puts it. If this was the block
        // in hand, the band is the landed block's to hold now, not its to drop.
        if (drop != null) _bandIsMine = false;
        _shift.jump(0);
        _lift.jump(0);
      } else {
        // Nothing was reordered, so whatever had stepped aside steps back.
        _lift.jump(0);
        _shift.go(0, motion.makeWay);
      }
    }
    _repaint();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final lift = _lift.value.clamp(0.0, 1.0);
    // The card a lifted block is cut to: its slot, a little smaller on every
    // side that is an edge of the card, so it reads as picked up off the
    // table rather than slid along it.
    final card = LiftCard(
      left: widget.side == LiftSide.trailing ? null : layerLiftInsetX * lift,
      right: widget.side == LiftSide.leading ? null : layerLiftInsetX * lift,
      insetY: layerLiftInsetY * lift,
      radius: liftRadius(t),
    );
    return Transform.translate(
      offset: Offset(0, _shift.value),
      child: CustomPaint(
        painter: lift <= 0
            ? null
            : LiftPainter(
                card: card,
                amount: lift,
                ground: liftGround(t),
                shadow: liftShadow(t),
              ),
        // Always in the tree and only cutting while the block is lifted, so
        // picking a row up never rebuilds what is inside it.
        child: ClipRRect(
          clipper: card,
          clipBehavior: lift <= 0 ? Clip.none : Clip.antiAlias,
          child: LayerLift(lifted: lift > 0, child: widget.child),
        ),
      ),
    );
  }
}

/// The shape a stretch of the lifted card is drawn in: its own box, in by
/// [insetY] top and bottom, and in by [left] and [right] at the ends, with
/// rounded corners where the shape has them. A null end is one that meets the
/// next stretch of the card: flush with the box and square.
class LiftCard extends CustomClipper<RRect> {
  final double? left;
  final double? right;
  final double insetY;
  final double radius;

  const LiftCard({
    required this.left,
    required this.right,
    required this.insetY,
    required this.radius,
  });

  @override
  RRect getClip(Size size) {
    final corner = Radius.circular(radius);
    return RRect.fromRectAndCorners(
      Rect.fromLTRB(
          left ?? 0, insetY, size.width - (right ?? 0), size.height - insetY),
      topLeft: left == null ? Radius.zero : corner,
      bottomLeft: left == null ? Radius.zero : corner,
      topRight: right == null ? Radius.zero : corner,
      bottomRight: right == null ? Radius.zero : corner,
    );
  }

  @override
  bool shouldReclip(LiftCard old) => old != this;

  @override
  bool operator ==(Object other) =>
      other is LiftCard &&
      other.left == left &&
      other.right == right &&
      other.insetY == insetY &&
      other.radius == radius;

  @override
  int get hashCode => Object.hash(left, right, insetY, radius);
}

/// The ground a block in hand is drawn on: an opaque card, so the rows it is
/// carried over do not show through it, with a faint shadow under it.
///
/// A shadow is for a thing that floats (docs/15-DESIGN.md §1.1), and a layer
/// in hand is one. Each stretch of the card casts its own, and at a flush end
/// the shadow is cast as if the card ran on and then cut off at the box, so
/// it meets the next stretch's at full strength instead of fading round a
/// corner that is not there.
class LiftPainter extends CustomPainter {
  final LiftCard card;
  final double amount;
  final Color ground;
  final Color shadow;

  const LiftPainter({
    required this.card,
    required this.amount,
    required this.ground,
    required this.shadow,
  });

  /// Further than either shadow reaches.
  static const double _run = 32;

  @override
  void paint(Canvas canvas, Size size) {
    final shape = card.getClip(size);
    final cast = RRect.fromLTRBAndCorners(
      shape.left - (card.left == null ? _run : 0),
      shape.top,
      shape.right + (card.right == null ? _run : 0),
      shape.bottom,
      topLeft: shape.tlRadius,
      bottomLeft: shape.blRadius,
      topRight: shape.trRadius,
      bottomRight: shape.brRadius,
    );
    canvas.save();
    canvas.clipRect(Rect.fromLTRB(
      card.left == null ? 0 : -_run,
      -_run,
      card.right == null ? size.width : size.width + _run,
      size.height + _run,
    ));
    // Two soft passes rather than one hard one: a wide faint one for the
    // height it has been lifted to, and a tight one that keeps its edge.
    canvas.drawRRect(
      cast.shift(const Offset(0, 4)),
      Paint()
        ..color = shadow.withValues(alpha: shadow.a * 0.7 * amount)
        ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 8),
    );
    canvas.drawRRect(
      cast.shift(const Offset(0, 1)),
      Paint()
        ..color = shadow.withValues(alpha: shadow.a * 0.5 * amount)
        ..maskFilter = const MaskFilter.blur(BlurStyle.normal, 1.5),
    );
    canvas.restore();
    // Opaque for as long as the card is there at all. The row on it has
    // handed its own fill over ([LayerLift]), so a ground that faded with the
    // lift would leave the row bare for the length of the landing.
    canvas.drawRRect(shape, Paint()..color = ground);
  }

  @override
  bool shouldRepaint(LiftPainter old) =>
      old.card != card ||
      old.amount != amount ||
      old.ground != ground ||
      old.shadow != shadow;
}

/// The stretch of the lifted card that crosses the gutter between the two
/// halves of the table, so a layer in hand is one card and not two.
///
/// Painted from [LayerDragState.band] by whatever owns the gutter. [top] and
/// [bottom] are the card's edges in the painter's own pixels.
class LiftBridgePainter extends CustomPainter {
  final double top;
  final double bottom;
  final double amount;
  final Color ground;
  final Color shadow;

  const LiftBridgePainter({
    required this.top,
    required this.bottom,
    required this.amount,
    required this.ground,
    required this.shadow,
  });

  @override
  void paint(Canvas canvas, Size size) {
    if (amount <= 0 || bottom <= top) return;
    canvas.save();
    canvas.clipRect(Offset.zero & size);
    canvas.translate(0, top);
    LiftPainter(
      card: const LiftCard(left: null, right: null, insetY: 0, radius: 0),
      amount: amount,
      ground: ground,
      shadow: shadow,
    ).paint(canvas, Size(size.width, bottom - top));
    canvas.restore();
  }

  @override
  bool shouldRepaint(LiftBridgePainter old) =>
      old.top != top ||
      old.bottom != bottom ||
      old.amount != amount ||
      old.ground != ground ||
      old.shadow != shadow;

  @override
  bool? hitTest(Offset position) => false;
}

/// The line that marks where a dragged row would land, for a drag that is
/// not carried: an accent rule across the seam the row would come to rest
/// against. [y] is that seam in the painter's own pixels.
class DropMarkPainter extends CustomPainter {
  final double y;
  final Color colour;

  const DropMarkPainter({required this.y, required this.colour});

  /// The same weight as the effect stack's drop line.
  static const double weight = 2;

  @override
  void paint(Canvas canvas, Size size) {
    // A seam scrolled out of view has no mark: the outline's overlay is
    // pinned over its rows and cuts nothing, so a line drawn past its edge
    // would land on the column header or the bar under the rows.
    if (y < 0 || y > size.height || size.height < weight) return;
    // Held inside the box, so the mark for the very top or the very bottom of
    // the stack is a whole line and not half of one.
    final at = y.clamp(weight / 2, size.height - weight / 2).toDouble();
    canvas.drawLine(
      Offset(0, at),
      Offset(size.width, at),
      Paint()
        ..color = colour
        ..strokeWidth = weight,
    );
  }

  @override
  bool shouldRepaint(DropMarkPainter old) =>
      old.y != y || old.colour != colour;

  @override
  bool? hitTest(Offset position) => false;
}

/// A column that paints one of its children last, so that child is drawn over
/// its neighbours wherever it has been moved to.
///
/// A column paints its children in order, so a block carried *down* the stack
/// would otherwise slide underneath every row after it. Only the paint order
/// changes: layout and hit-testing are a plain column's.
class RaisedColumn extends Column {
  /// The child to paint last, as an index into [children], or null for none.
  final int? raised;

  const RaisedColumn({
    super.key,
    this.raised,
    super.crossAxisAlignment,
    super.children,
  });

  @override
  RenderFlex createRenderObject(BuildContext context) => _RenderRaisedFlex(
        direction: direction,
        mainAxisAlignment: mainAxisAlignment,
        mainAxisSize: mainAxisSize,
        crossAxisAlignment: crossAxisAlignment,
        textDirection: getEffectiveTextDirection(context),
        verticalDirection: verticalDirection,
        textBaseline: textBaseline,
        clipBehavior: clipBehavior,
        spacing: spacing,
        raised: raised,
      );

  @override
  void updateRenderObject(BuildContext context, RenderFlex renderObject) {
    super.updateRenderObject(context, renderObject);
    (renderObject as _RenderRaisedFlex).raised = raised;
  }
}

class _RenderRaisedFlex extends RenderFlex {
  _RenderRaisedFlex({
    super.direction,
    super.mainAxisAlignment,
    super.mainAxisSize,
    super.crossAxisAlignment,
    super.textDirection,
    super.verticalDirection,
    super.textBaseline,
    super.clipBehavior,
    super.spacing,
    int? raised,
  }) : _raised = raised;

  int? get raised => _raised;
  int? _raised;
  set raised(int? value) {
    if (value == _raised) return;
    _raised = value;
    markNeedsPaint();
  }

  @override
  void paint(PaintingContext context, Offset offset) {
    final raised = _raised;
    if (raised == null) {
      super.paint(context, offset);
      return;
    }
    RenderBox? held;
    var child = firstChild;
    for (var i = 0; child != null; i++) {
      final parentData = child.parentData! as FlexParentData;
      if (i == raised) {
        held = child;
      } else {
        context.paintChild(child, parentData.offset + offset);
      }
      child = parentData.nextSibling;
    }
    if (held != null) {
      context.paintChild(
          held, (held.parentData! as FlexParentData).offset + offset);
    }
  }
}
