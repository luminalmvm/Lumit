// The block tools' arithmetic: what a marquee's catch of keyframes measures,
// where a stretch handle puts each of them, and where Reverse and Stagger do
// (K-458, docs/15 §12A.1a).
//
// In plain terms: select several keyframes and they become one *block* — a box
// with a handle at each end and a badge saying how many keys it holds and how
// many frames it spans. Drag a handle and the block stretches: the end you did
// not touch stays put, the end you dragged goes where you put it, and every key
// between them slides to keep its share of the span. Reverse turns the block
// back to front in time, values travelling with their keys. Stagger fans them
// out by a fixed number of frames.
//
// Pure, so all of it unit-tests without a widget tree and none of it crosses
// the bridge — the same bargain `graph_maths.dart` and `easing_curve.dart`
// strike. Everything here speaks in **frames**, fractional, because a key may
// sit between frames with the magnet off (docs/07 §4.5).

/// The smallest span a stretch may squeeze a block into, in frames.
///
/// Not zero: two keys cannot share a time — the engine refuses a curve whose
/// times do not strictly ascend — so a handle dragged onto its anchor would
/// collapse the block into a write that is simply refused, and the gesture
/// would read as broken rather than as bounded. One frame is the narrowest
/// block that can still be written on any row, whatever the magnet says.
const double minBlockSpan = 1.0;

/// A block of selected keyframes, measured (K-458).
///
/// [first] and [last] are the earliest and latest frames the selection reaches;
/// [count] is how many keys it holds. The badge reads these two numbers, and
/// so does the box: the box spans [first] to [last], and a handle sits at each.
class KeyBlock {
  final double first;
  final double last;
  final int count;

  const KeyBlock({
    required this.first,
    required this.last,
    required this.count,
  });

  /// The block's span in frames — the badge's second number.
  ///
  /// Rounded, because the badge counts frames and a block ending three
  /// thousandths of a frame short of 24 is a block 24 frames long that has been
  /// through a rational conversion.
  int get spanFrames => (last - first).round();

  /// Whether there is a block at all. One key is a key, not a block: it has its
  /// own drag already, and a box round it would say "0 f", which is nothing to
  /// take hold of.
  static bool isBlock(int count) => count >= 2;
}

/// A block stretch in flight: which keys it moves, the end that stays put and
/// the end that is being dragged.
///
/// Held by the panel and read by every lane, the way a bar drag is (K-208): the
/// keys are spread across rows in two scroll views, and a stretch that only the
/// handle knew about would move the box while the diamonds sat still.
class KeyStretch {
  /// The keys the gesture moves, as `rowId#index` — the block's own selection,
  /// captured when the handle was taken hold of so that it cannot change
  /// underneath the drag.
  final Set<String> keys;

  /// The end that stays put, in frames.
  final double anchor;

  /// Where the dragged end started, and where it is now.
  final double from;
  final double to;

  const KeyStretch({
    required this.keys,
    required this.anchor,
    required this.from,
    required this.to,
  });

  KeyStretch movedTo(double to) =>
      KeyStretch(keys: keys, anchor: anchor, from: from, to: to);

  /// The scale the block is being multiplied by: how much of the original span
  /// the dragged span now is.
  ///
  /// A block whose two ends started on the same frame has no span to scale, and
  /// dividing by it is how a stretch becomes infinities — it reports 1, which
  /// leaves every key where it is.
  double get scale {
    final was = from - anchor;
    if (was == 0) return 1;
    return (to - anchor) / was;
  }

  /// Where [frame] lands under this stretch: its distance from the anchor,
  /// scaled.
  ///
  /// **Whole-frame snapped when [whole]**, exactly as a single key's drag is
  /// with the magnet on — the block is a gesture on keys, not a new kind of
  /// thing, and a stretch that left every key on a fraction of a frame while a
  /// drag of one landed on a whole one would be two rules for one act.
  double frameOf(double frame, {required bool whole}) {
    final moved = anchor + (frame - anchor) * scale;
    if (!whole) return moved;
    // **The dragged end lands exactly where the drag put it.** [to] is the
    // handle's own answer, and the handle snaps to the shared targets — a
    // marker, the playhead, another row's key — which need not sit on a whole
    // frame. Rounding it as well would pull the end back off the target it had
    // just been caught by, so the snap would show and then not stick.
    // Everything between still rounds, exactly as one key's drag does.
    if ((moved - to).abs() < 1e-9) return moved;
    return moved.roundToDouble();
  }
}

/// [to] clamped so the block keeps at least [minBlockSpan] frames and never
/// turns inside out through its anchor.
///
/// Without it a handle dragged past its anchor would invert the block — every
/// key's order reversed, which is Reverse's job and not a stretch's — and one
/// dragged exactly onto it would ask for a curve the engine must refuse.
double clampStretch({
  required double anchor,
  required double from,
  required double to,
}) {
  // Which side the dragged end was on decides which side it stays on.
  if (from >= anchor) {
    return to < anchor + minBlockSpan ? anchor + minBlockSpan : to;
  }
  return to > anchor - minBlockSpan ? anchor - minBlockSpan : to;
}

/// [frames] mirrored within their own span: the first key's time goes to the
/// last's and back, so the block plays backwards while staying exactly where it
/// was on the Timeline.
///
/// Returned in the same order as it was given, so a caller can pair each new
/// time with the key it belongs to — **the value travels with its key**, which
/// is what makes this a reverse rather than a re-ordering of values under
/// fixed times.
List<double> reversedFrames(List<double> frames) {
  if (frames.isEmpty) return const [];
  var lo = frames.first;
  var hi = frames.first;
  for (final f in frames) {
    if (f < lo) lo = f;
    if (f > hi) hi = f;
  }
  final sum = lo + hi;
  return [for (final f in frames) sum - f];
}

/// Which way a stagger fans a block out (K-458, the Ease popover's own control).
enum StaggerOrder {
  /// The first row's keys stay put and each row below is pushed later.
  topDown,

  /// The last row's keys stay put and each row above is pushed later.
  bottomUp,
}

/// [frames] pushed later by [step] frames per rank, so a run of rows arrives one
/// after another rather than together.
///
/// [rank] is the row's place in the block, counted from the top; [rows] is how
/// many rows the block covers. Bottom-up counts the same ranks from the other
/// end, so the two orders are one arithmetic and not two.
///
/// A [step] of zero is the identity, which is what makes zero the resting value
/// of the popover's field rather than a special case anywhere else.
double staggeredFrame(
  double frame, {
  required int rank,
  required int rows,
  required double step,
  required StaggerOrder order,
}) {
  final place = order == StaggerOrder.topDown ? rank : (rows - 1 - rank);
  return frame + place * step;
}
