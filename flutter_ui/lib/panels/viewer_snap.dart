// Snapping on the picture (K-689, docs/07 §2.2 item 6).
//
// **In plain terms.** While you drag a layer about the Viewer it should want to
// land on the lines already there — a guide you pulled out of a ruler, a line of
// the grid — rather than a hair away from one. That wanting is the same magnet
// the Timeline has, turned ninety degrees: `timeline_snap.dart` snaps a *time*
// to the things on a time ruler, and this snaps a *place* to the things on a
// picture. The two share their grammar deliberately, and the constants with it:
//
// *Distance is measured in screen pixels, never in comp units.* Zoomed out, a
// hundred comp pixels may be ten on screen and the magnet should be eager;
// zoomed in, one comp pixel may be ten and it should not reach across three of
// them. Measuring on screen makes the magnification the precision control.
//
// *What caught the drag is reported, not just where it landed*, so the caller
// can draw the line that took it — a drag that jumps with nothing to show for
// it looks like a fault rather than a service.
//
// *`Ctrl` suspends it* — `snapSuspended`, shared with the Timeline, so the one
// escape hatch is the same key everywhere.
//
// All of it is pure, so all of it is tested against hand-computed cases rather
// than by dragging in a widget tree.

import 'dart:ui' show Offset, Rect, Size;

import '../state/workspace.dart' show ViewerGuide;
import 'timeline_snap.dart' show snapSlopPixels;
import 'viewer_rulers.dart' show viewerGuideScreen;

/// What a drag on the picture landed on: how far it was pulled, in screen
/// pixels, and the line that pulled it — null when nothing did, which is what
/// tells the caller to draw no indicator.
typedef ViewerSnap = ({double shift, double? caught});

/// Nothing reached for it.
const ViewerSnap noViewerSnap = (shift: 0.0, caught: null);

/// How far one axis of a drag has to move for the nearest target to take it.
///
/// [moving] are the edges being dragged, on screen — for a layer, its box's two
/// edges and its middle, because any of the three is a thing somebody lines up.
/// [targets] are the lines it can land on, on screen. The pair that are nearest
/// wins, and only if they are within [slopPx] of each other.
ViewerSnap snapToLines({
  required Iterable<double> moving,
  required Iterable<double> targets,
  double slopPx = snapSlopPixels,
}) {
  double? caught;
  var shift = 0.0;
  var best = slopPx;
  for (final target in targets) {
    if (!target.isFinite) continue;
    for (final edge in moving) {
      if (!edge.isFinite) continue;
      final gap = target - edge;
      // Strictly nearer, so the first of two equally close targets keeps it
      // and the answer does not depend on the order they were gathered in.
      if (gap.abs() < best) {
        best = gap.abs();
        shift = gap;
        caught = target;
      }
    }
  }
  return (shift: shift, caught: caught);
}

/// Every line a drag on the picture can land on, on screen (docs/07 §2.2 item
/// 6): the comp's **guides**, and — only while *Snap to grid* is ticked — the
/// **grid**'s own lines and the frame's four edges.
///
/// [vertical] picks the axis: the lines a horizontal movement lands on (comp
/// x), or the ones a vertical movement lands on (comp y).
///
/// The grid is the same eighths `ViewerOverlayPainter` draws, so what a drag
/// lands on is what the picture shows — which is the whole reason the count
/// lives in one constant and is passed in here rather than repeated.
List<double> viewerSnapTargets({
  required Iterable<ViewerGuide> guides,
  required bool vertical,
  required Rect picture,
  required Size compSize,
  bool grid = false,
  int divisions = 8,
}) {
  final out = <double>[
    for (final guide in guides)
      if (guide.vertical == vertical)
        viewerGuideScreen(guide, picture: picture, compSize: compSize),
  ];
  if (grid && divisions > 0) {
    final from = vertical ? picture.left : picture.top;
    final span = vertical ? picture.width : picture.height;
    for (var i = 0; i <= divisions; i++) {
      out.add(from + span * i / divisions);
    }
  }
  return out;
}

/// How far a drag of [box] should be nudged so it lands on something.
///
/// [box] is where the dragged thing already is on screen, [delta] the travel
/// the pointer has asked for; the answer is the travel to actually use. Each
/// axis is decided on its own, because a layer lined up with a guide down one
/// side is still free to move along it — which is what a guide is *for*.
Offset snapViewerDrag({
  required Rect box,
  required Offset delta,
  required List<double> verticals,
  required List<double> horizontals,
  double slopPx = snapSlopPixels,
}) {
  final moved = box.shift(delta);
  final x = snapToLines(
    moving: [moved.left, moved.center.dx, moved.right],
    targets: verticals,
    slopPx: slopPx,
  );
  final y = snapToLines(
    moving: [moved.top, moved.center.dy, moved.bottom],
    targets: horizontals,
    slopPx: slopPx,
  );
  return delta + Offset(x.shift, y.shift);
}
