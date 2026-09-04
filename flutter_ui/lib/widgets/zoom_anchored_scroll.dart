// A horizontal scroll that holds one frame still while the thing it scrolls
// grows underneath it (docs/07-UI-SPEC.md §4.6).
//
// **In plain terms.** Zooming the Timeline widens the lanes. If the scroll
// offset stayed where it was, whatever you were looking at would slide off to
// the right, so the offset has to be corrected to match the new width — every
// frame of the zoom, because the lanes are growing the whole way through.
//
// **Why that correction belongs here and not in the panel.** The obvious way is
// to jump the scroll controller the moment the zoom changes. That works, but it
// jumps to an offset that is only valid for the *new* width — and the new width
// has not been laid out yet. For the rest of that frame the scroll position is
// past its own end, so Flutter starts springing it back, and the scrollbar's
// thumb is drawn from a position and a length that disagree. The result is a
// thumb that twitches all the way through a zoom drag. Reported by the owner as
// the scrollbar "jumping around a bit".
//
// A scroll position is told its new content size during layout, in
// `applyContentDimensions`, and that is the one moment where the width and the
// offset can be made to agree. Correcting there is not a workaround: the method
// is *documented* to return false when it has moved the offset, so that layout
// runs again with the corrected value. Nothing outside that pass ever sees an
// offset that does not fit its content.

import 'package:flutter/widgets.dart';

/// What a zoom is holding still: a frame, and where on screen it should stay.
///
/// [frames] is how many frames the whole scrollable content spans, which is what
/// turns the two into pixels once the content's width is known.
@immutable
class ZoomAnchor {
  const ZoomAnchor({
    required this.frame,
    required this.viewportX,
    required this.frames,
    this.pad = 0,
  });

  /// The frame to keep under [viewportX].
  final double frame;

  /// Where on screen — measured from the viewport's left edge — that frame
  /// should stay.
  final double viewportX;

  /// How many frames the content spans end to end.
  final int frames;

  /// The pixels of padding at each end of the content that the frames do
  /// *not* occupy (`TimelineAxis.pad`). Passed in rather than assumed, so this
  /// arithmetic stays a pure function of what it is told; without it every
  /// anchor drifts by up to a padding's width, worst at the two ends.
  final double pad;

  @override
  bool operator ==(Object other) =>
      other is ZoomAnchor &&
      other.frame == frame &&
      other.viewportX == viewportX &&
      other.frames == frames &&
      other.pad == pad;

  @override
  int get hashCode => Object.hash(frame, viewportX, frames, pad);
}

/// A [ScrollController] that can be asked to hold a frame still through the
/// next layout.
///
/// The anchor is **one-shot**: it is spent by the layout that follows, so an
/// ordinary scroll — a wheel, a drag of the thumb — is never pulled back to
/// where a zoom once was. A zoom that is still moving simply asks again on its
/// next tick, which it does anyway because every tick is a new width.
class ZoomAnchoredScrollController extends ScrollController {
  ZoomAnchor? _anchor;

  /// Hold [anchor] through the next layout.
  void hold(ZoomAnchor anchor) => _anchor = anchor;

  /// The anchor waiting to be applied, if any. Visible for tests and for a
  /// caller that wants to know whether a zoom is in hand.
  ZoomAnchor? get anchor => _anchor;

  /// Forget the anchor without applying it.
  void release() => _anchor = null;

  @override
  ScrollPosition createScrollPosition(
    ScrollPhysics physics,
    ScrollContext context,
    ScrollPosition? oldPosition,
  ) =>
      _ZoomAnchoredScrollPosition(
        physics: physics,
        context: context,
        oldPosition: oldPosition,
        owner: this,
      );
}

/// Where the offset has to be for [anchor]'s frame to land on its point, given
/// the content the viewport has just been told about.
///
/// Pure, so the arithmetic is tested without a viewport: the content is the
/// viewport plus everything that scrolls past it, which divided by the frame
/// count is what one frame is worth in pixels. Clamped, because the ends of the
/// content are the ends: at fit there is nowhere to scroll to, and an anchor
/// near either end is honoured as far as the content allows.
double zoomAnchorOffset(
  ZoomAnchor anchor, {
  required double viewportDimension,
  required double minScrollExtent,
  required double maxScrollExtent,
}) {
  if (anchor.frames <= 0) return minScrollExtent;
  final content = viewportDimension + maxScrollExtent;
  final span = content - anchor.pad * 2;
  final perFrame = (span < 0 ? 0 : span) / anchor.frames;
  return (anchor.pad + anchor.frame * perFrame - anchor.viewportX)
      .clamp(minScrollExtent, maxScrollExtent)
      .toDouble();
}

class _ZoomAnchoredScrollPosition extends ScrollPositionWithSingleContext {
  _ZoomAnchoredScrollPosition({
    required super.physics,
    required super.context,
    super.oldPosition,
    required this.owner,
  });

  final ZoomAnchoredScrollController owner;

  @override
  bool applyContentDimensions(double minScrollExtent, double maxScrollExtent) {
    final settled =
        super.applyContentDimensions(minScrollExtent, maxScrollExtent);
    final anchor = owner.anchor;
    if (anchor == null || anchor.frames <= 0 || !hasViewportDimension) {
      return settled;
    }
    // Spent whether or not it moves anything: an anchor left armed would be
    // applied by the next unrelated layout — a window resize, say — and pull
    // the view back to a zoom the reader has since scrolled away from.
    owner.release();
    final want = zoomAnchorOffset(
      anchor,
      viewportDimension: viewportDimension,
      minScrollExtent: minScrollExtent,
      maxScrollExtent: maxScrollExtent,
    );
    // A hair's difference is not worth another layout pass.
    if ((want - pixels).abs() < 0.01) return settled;
    correctPixels(want);
    // False, so the viewport lays out again with the offset it now has. The
    // anchor is spent, so that pass falls through to `super` and settles.
    return false;
  }
}
