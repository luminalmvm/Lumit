// The Timeline's shared numbers: the column and keyframe metrics, the zoom
// step, the layer-drag model and the row list both halves of the table are
// built from.
//
// The sampled-scalar cache moved to state/comp_time.dart, beside the frame↔time
// memory it was always modelled on — the Effect controls panel's rows want the
// same batching, and reaching into the Timeline's own metrics for it would have
// been the wrong way round.
//
// Split out of timeline_panel_frb.dart, which the outline, the lanes and the
// panel itself all read these from.

import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import '../l10n/strings.dart';
import '../state/timeline_columns.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'graph_panel.dart' show DrivenParam;
import 'layer_fold_frb.dart';
import 'timeline_group_row_frb.dart';
import 'package:lumit_flutter/src/rust/api/retime.dart';

/// The layer-number column: the mockup's own 18 (K-451), shared by the column
/// header's `#` and the muted mono number under it so the two stack.
const double numberCellWidth = 18;

/// How thick a scrollbar thumb is — 7 (K-451, docs/15 §12A.6, where it is
/// named for the graph side's horizontal bar; the lanes' bar carries the same
/// one, so the two modes do not swap bar shapes on the switch).
///
/// **The vertical thumbs in the gutters are the same 7** (§6.15). They were
/// whatever [scrollGutterWidth] left after a 3px margin each side, which came
/// out 6 — one pixel thinner than the bar along the bottom of the very same
/// view, for no reason beyond the two being written in different units. The
/// gutter keeps its width; the thumb is centred in it.
const double scrollbarThickness = 7;

/// Where something [size] across starts when it is centred in [extent] **on
/// whole pixels**: an odd remainder is floored rather than split.
///
/// Half a pixel off centre cannot be seen; half a pixel across the grid can.
/// A filled block draws its edge grey ([scrollbarThickness] in a 12px gutter,
/// §6.15) and a glyph's strokes smear — and where the glyph carries the icon
/// set's own half-pixel nudge onto the grid (K-456), a fractional base moves
/// it back off again, which is the switch column's drift (§6.20).
double wholePixelInset(double extent, double size) =>
    ((extent - size) / 2).floorToDouble().clamp(0.0, extent);

/// Half a keyframe's height on a property's own lane — **the same in Layers
/// mode and in Keys mode** (K-459). The drawing measures the key 11px point
/// to point in both, so this is 5.5. Layers mode used to draw them at 8: a
/// mark you take hold of should not shrink because there happen to be bars
/// beside it, and the two modes showing the same key at two sizes made the
/// switch between them look like a change to the comp.
const double laneKeyHalf = 5.5;

/// How a key is drawn: the shape says its interpolation (§12A.1, K-457 —
/// **diamond linear, hourglass bezier, square hold**), which is what lets a
/// lane of keys be read rather than merely counted.
enum KeyShape { diamond, hourglass, square }

/// One side of a key's interpolation as a shape. Each side answers for its own
/// half of the mark (K-457), so nothing here has to choose between them: a key
/// that eases in and holds out is half hourglass and half square, which is the
/// truth about it and was previously unsayable.
/// An **automatic** side is an eased one — the hourglass half — because that
/// is what the shape says: the movement is curved through this key. Which of
/// the three tangent modes shaped the curve is not a lane's business; the mark
/// says how the motion runs, not who decided it.
KeyShape keyShapeOfSide(BridgeSideInterp side) => switch (side) {
      BridgeSideInterp_Hold() => KeyShape.square,
      BridgeSideInterp_Bezier() ||
      BridgeSideInterp_Auto() =>
        KeyShape.hourglass,
      _ => KeyShape.diamond,
    };

/// [key]'s two halves: the shape of the interpolation coming **in** to it, and
/// of the one going **out**. The mark is split at its vertical centre and each
/// half drawn from its own side.
(KeyShape, KeyShape) keyShapeOf(BridgeKeyframe key) =>
    (keyShapeOfSide(key.interpIn), keyShapeOfSide(key.interpOut));

/// One side of one key's mark: [shape]'s left half when [left], its right half
/// otherwise, standing [half] above and below [mid] and split down the line at
/// [x] (K-457).
///
/// **Every shape is the same height** — that is what makes a lane of mixed
/// keys read as one row of marks rather than as a row that also changes size.
/// Only the diamond is as wide as it is tall; the other two take the drawing's
/// narrower 8-in-11 box, which is what keeps a hold key from reading as a
/// diamond that failed to turn.
///
/// Top level, and returning the path rather than drawing it, so the geometry
/// can be asked questions directly: what a mark's halves are is a claim about
/// shapes, and reading it back out of a rendered lane would be measuring the
/// renderer.
Path keyHalfPath(KeyShape shape, double x, double mid, double half,
    {required bool left}) {
  final w =
      (shape == KeyShape.diamond ? half : half * 8 / 11) * (left ? -1.0 : 1.0);
  return switch (shape) {
    KeyShape.diamond => Path()
      ..moveTo(x, mid - half)
      ..lineTo(x + w, mid)
      ..lineTo(x, mid + half)
      ..close(),
    // A square stood square, not on its corner.
    KeyShape.square => Path()
      ..addRect(Rect.fromLTRB(
          left ? x + w : x, mid - half, left ? x : x + w, mid + half)),
    // Two triangles tip to tip: half an hourglass is still two triangles,
    // meeting at the centre point the whole mark is split on. Two subpaths
    // rather than one outline, because a polygon drawn through its own pinch
    // point fills the wrong side of itself.
    KeyShape.hourglass => Path()
      ..moveTo(x + w, mid - half)
      ..lineTo(x, mid - half)
      ..lineTo(x, mid)
      ..close()
      ..moveTo(x + w, mid + half)
      ..lineTo(x, mid + half)
      ..lineTo(x, mid)
      ..close(),
  };
}

/// Half a key's width, per shape: only the diamond is as wide as it is tall.
double _keyHalfWidth(KeyShape shape, double half) =>
    shape == KeyShape.diamond ? half : half * 8 / 11;

/// The outer boundary of [shape]'s half, walked from the mark's **top centre**
/// down its own side to the mark's **bottom centre**. The two ends are on the
/// centre line; nothing between them is, save an hourglass's pinch.
List<Offset> _keyHalfOutline(KeyShape shape, double x, double mid, double half,
    {required bool left}) {
  final w = _keyHalfWidth(shape, half) * (left ? -1.0 : 1.0);
  return switch (shape) {
    KeyShape.diamond => [
        Offset(x, mid - half),
        Offset(x + w, mid),
        Offset(x, mid + half),
      ],
    KeyShape.square => [
        Offset(x, mid - half),
        Offset(x + w, mid - half),
        Offset(x + w, mid + half),
        Offset(x, mid + half),
      ],
    KeyShape.hourglass => [
        Offset(x, mid - half),
        Offset(x + w, mid - half),
        Offset(x, mid),
        Offset(x + w, mid + half),
        Offset(x, mid + half),
      ],
  };
}

/// A whole key's mark as **one path with no interior edge on the centre line**
/// — the thing that is actually painted (§5 of docs/impl/timeline-interaction).
///
/// Drawing the two halves as two paths looked right in the geometry and wrong
/// on the screen: the renderer anti-aliases each path against the ground on its
/// own, so along the shared centre line two half-covered edges met and the
/// ground showed through the pair as a hairline seam down the middle of every
/// mark — including the plain diamonds, where there is no seam to draw at all.
///
/// So the pair is composed before it is painted. A **same-shape** pair is
/// simply the whole shape's own contour. A **mixed** pair is the union of the
/// two halves as one contour: down the left half's outer boundary from top
/// centre, back up the right half's, crossing the centre line only at the top
/// and the bottom (and at an hourglass half's pinch, where the contour turns
/// through a point rather than drawing along the line).
///
/// The **hourglass/hourglass** pair keeps its two triangles: they meet at one
/// point, and two subpaths of one path that share a point draw no seam.
/// [keyHalfPath] stays as the geometry oracle — what each half *is* — while
/// this is what the canvas gets.
Path keyMarkPath((KeyShape, KeyShape) pair, double x, double mid, double half) {
  final (into, out) = pair;
  if (into == out) {
    final w = _keyHalfWidth(into, half);
    return switch (into) {
      KeyShape.diamond => Path()
        ..moveTo(x, mid - half)
        ..lineTo(x + w, mid)
        ..lineTo(x, mid + half)
        ..lineTo(x - w, mid)
        ..close(),
      KeyShape.square => Path()
        ..addRect(Rect.fromLTRB(x - w, mid - half, x + w, mid + half)),
      KeyShape.hourglass => Path()
        ..moveTo(x - w, mid - half)
        ..lineTo(x + w, mid - half)
        ..lineTo(x, mid)
        ..close()
        ..moveTo(x - w, mid + half)
        ..lineTo(x + w, mid + half)
        ..lineTo(x, mid)
        ..close(),
    };
  }
  final down = _keyHalfOutline(into, x, mid, half, left: true);
  // Reversed, so it walks bottom centre back up to top centre; its first point
  // is the left half's last and its last is the moveTo, so both are dropped.
  final up = _keyHalfOutline(out, x, mid, half, left: false).reversed.toList();
  final path = Path()..moveTo(down.first.dx, down.first.dy);
  for (final p in [...down.skip(1), ...up.sublist(1, up.length - 1)]) {
    path.lineTo(p.dx, p.dy);
  }
  return path..close();
}

/// How much one step of the zoom is worth — a press of `=` / `-`, or a click
/// on one of the landscapes flanking the slider (§6.5). A doubling, which is
/// what After Effects' own keys do: the wheel's gentler notch is for a hand
/// that can keep rolling, and a discrete step is one jump.
const double zoomKeyStep = 2;

/// The zoom one step [inward] (or out) of [zoom], within the range the slider
/// covers — the whole composition at 1, [maxZoom] at the far end.
double zoomNudged(double zoom,
        {required bool inward, required double maxZoom}) =>
    (inward ? zoom * zoomKeyStep : zoom / zoomKeyStep)
        .clamp(1.0, maxZoom < 1 ? 1.0 : maxZoom);

/// A layer drag in flight: the index lifted, and the index it would land on.
///
/// **Held by the panel and read by both halves of the table**, which is the
/// point (K-208). The outline owns the gesture — the name is the stack handle
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

/// One layer as **both halves of the table see it**: the rows it shows, the
/// room an open Sequence view wants, and the height those come to.
///
/// The outline and the lane area are still two widget trees, and they have to
/// be: they sit in two scroll views, and only the lane half rebuilds when the
/// zoom moves (K-293). What they must never be is two *opinions*. Each half
/// used to walk [layerFoldRows] for itself, test `open` for itself and look up
/// `sequenceExtra` for itself, and the table was level only because all three
/// pairs happened to agree — a layer could grow a row on one side and not the
/// other, and nothing would say so. Decided once here, the halves can differ in
/// what they draw but not in what a layer *is*.
class LayerRow {
  final BridgeLayerEntry entry;

  /// The layer's id as a string, which is the key everything else is filed
  /// under — worked out once rather than at each of the dozen places that
  /// wanted it.
  final String id;

  /// Twirled open — whether [foldRows] are **drawn**.
  final bool open;

  /// The rows this layer's fold-out has, open or shut.
  ///
  /// Held for a shut layer too, because snapping wants them: a keyframe is
  /// somewhere in time whether or not its row is on screen, and a key drag or
  /// a razor cut can land on it either way (`_snapTargets`, K-292). Everything
  /// that *draws* reads [drawnRows] instead — the one place that difference is
  /// written down, rather than an `open` test remembered at each of five call
  /// sites.
  final List<LayerFoldRow> foldRows;

  /// The rows the table draws under this layer: its fold-out when it is
  /// twirled open, nothing when it is not.
  List<LayerFoldRow> get drawnRows => open ? foldRows : const [];

  /// How much taller an open Sequence view makes this row (K-248), or null
  /// when the layer has no view open. **The outline reserves exactly the room
  /// the lanes draw the view in**, or every row below sits at a different
  /// height on the two sides.
  final double? sequenceExtra;

  /// Whether this layer's source carries sound, and whether it has a picture
  /// to show (K-435). What the switches column reads to draw only the switches
  /// this layer can actually use: no audible switch on a solid that has never
  /// made a sound, no visibility switch on a music track that has never shown
  /// anything.
  ///
  /// Carried here, decided once by the panel, because answering either means
  /// probing the media — the cost K-184 exists to keep out of a row's build.
  final bool hasAudio;
  final bool hasPicture;

  /// What one row of this layer measures — `t.density.laneRow` (K-454).
  ///
  /// **Carried, not looked up.** This is a plain description of a row, built
  /// once for the whole panel and read by the maths that has no `BuildContext`
  /// to ask: the row seams, the box-select catch, the drag slots. Handing it
  /// the number at the one place a row is made keeps the density in the theme
  /// where it belongs, and keeps every reader of a row answering with the same
  /// arithmetic.
  final double rowHeight;

  /// Every keyframe anywhere on this layer, for the diamonds its **own** row
  /// draws while it is shut (§12A.1). Empty while the layer is open: each
  /// property then draws its own on its own lane, and saying it twice would
  /// put a small diamond behind every large one.
  final List<BridgeKeyframe> summaryKeys;

  /// The **layer group's header row**, when this layer is the topmost member of
  /// one (K-702), or null for every other layer.
  ///
  /// Carried on the carrier layer rather than standing as a row of its own,
  /// which is what let groups arrive without touching the drag arithmetic, the
  /// row seams or either half's window: `rows` is still one entry per visible
  /// layer, and the header is simply drawn above its carrier's row inside the
  /// carrier's own block.
  final GroupHeader? groupHeader;

  const LayerRow({
    required this.entry,
    required this.id,
    required this.open,
    required this.foldRows,
    required this.rowHeight,
    this.summaryKeys = const [],
    required this.sequenceExtra,
    this.hasAudio = false,
    this.hasPicture = true,
    this.groupHeader,
  });

  /// Whether the layer's **own** row draws at all. A shut group's carrier
  /// draws the header and nothing else — its bar, its switches and its
  /// fold-out are what the fold hid, along with the members below it.
  bool get bodyDrawn => !(groupHeader?.folded ?? false);

  /// This block's height: the group header it carries, then — unless the fold
  /// is shut — its own row, the rows it draws, and its open view.
  double get height =>
      (groupHeader == null ? 0 : rowHeight) +
      (bodyDrawn
          ? rowHeight * (1 + drawnRows.length) + (sequenceExtra ?? 0)
          : 0);
}

/// Where a comp's groups land on its rows: the header each carrier layer
/// draws, and the layers a shut fold takes off the list.
///
/// **Both answers in one walk, decided once for the panel.** The two are the
/// same question asked from either end — which layer carries a band, and which
/// layers that band swallowed — and answering them apart is how the outline and
/// the lanes end up disagreeing about how many rows a comp has.
///
/// The engine has already resolved each group to the unbroken run it actually
/// draws over (`BridgeLayerGroup.members`, in stack order), so there is no
/// membership arithmetic here at all: the first member carries the header, and
/// a shut fold hides every member including that one's own row.
({Map<String, GroupHeader> headers, Set<String> hidden}) groupFolds({
  required List<BridgeLayerGroup> groups,
  required Set<String> folded,
}) {
  final headers = <String, GroupHeader>{};
  final hidden = <String>{};
  for (final g in groups) {
    if (g.members.isEmpty) continue;
    final shut = folded.contains(g.id.toString());
    headers[g.members.first.toString()] = GroupHeader(g, shut);
    if (shut) {
      // Every member but the first: the first stays in the list as the row
      // the header is drawn on, with its own body standing down.
      for (final m in g.members.skip(1)) {
        hidden.add(m.toString());
      }
    }
  }
  return (headers: headers, hidden: hidden);
}

/// What a column group is called — in its header, and on the bottom bar's
/// toggle for it (K-448), which must name the same thing the header does.
/// **The same words the column headers carry** (§12A.1): Switches, Modes,
/// Parent — the mockup's own bottom-bar row.
String columnGroupLabel(TimelineGroup group) => switch (group) {
      TimelineGroup.switches => l10n.columnSwitches,
      TimelineGroup.identity => l10n.columnLayer,
      TimelineGroup.render => l10n.columnModes,
      TimelineGroup.compose => l10n.columnCompose,
      TimelineGroup.parent => l10n.columnParent,
      TimelineGroup.timings => l10n.tipRenderTime,
    };

/// Decide every layer's row, once for the whole panel. `flowParams` and
/// `volumeDb` are the panel's once-per-revision reads, riding down onto the
/// fold rows (K-184).
///
/// [reveal] names the layers drawn **filtered**, and by which rule: each builds
/// its fold-out as though every twirl in it were down, and then keeps only the
/// rows that answer ([revealFoldRows]). A layer with nothing qualifying comes
/// back shut.
///
/// Three things ask for it. The **Animated filter** (K-441, 6.43) asks for the
/// whole comp, and passes [everyLayerKeyframed]. A single **`U`** asks for the
/// layers it just revealed and no others (K-622), so that opening one layer's
/// keyed rows does not quietly filter every other layer on the panel. The
/// **Animation ▸ Reveal** rows (K-684) ask the same of the selection, with the
/// wider filters. [compWidth]/[compHeight] ride along for the widest of them,
/// which is the only one that needs to know where an unmoved layer sits.
List<LayerRow> layerRows({
  required List<BridgeLayerEntry> layers,
  required Set<String> open,
  required double rowHeight,
  required Map<String, bool> hasAudio,
  Map<String, bool> hasPicture = const {},
  Map<String, double> sequenceExtra = const {},
  Map<String, BridgeFlowParams> flowParams = const {},
  Map<String, BridgeScalar> volumeDb = const {},
  Map<String, Map<String, DrivenParam>> driven = const {},
  Map<String, RevealFilter> reveal = const {},
  Map<String, GroupHeader> groupHeaders = const {},
  double compWidth = 0,
  double compHeight = 0,
}) {
  final out = <LayerRow>[];
  for (final entry in layers) {
    final id = entry.layer.internallayerId.toString();
    final filter = reveal[id];
    final built = layerFoldRows(
        entry: entry,
        open: filter != null ? everyFoldPath : open,
        hasAudio: hasAudio[id] ?? false,
        flowParams: flowParams[id],
        volumeDb: volumeDb[id],
        driven: driven[id] ?? const {});
    final fold = filter == null
        ? built
        : revealFoldRows(built, filter,
            compWidth: compWidth, compHeight: compHeight);
    final isOpen = filter != null ? fold.isNotEmpty : open.contains(id);
    out.add(LayerRow(
      entry: entry,
      id: id,
      open: isOpen,
      rowHeight: rowHeight,
      foldRows: fold,
      // Only for a shut layer: an open one shows the real thing.
      summaryKeys: isOpen
          ? const []
          : layerKeys(
              entry: entry,
              flowParams: flowParams[id],
              volumeDb: volumeDb[id],
            ),
      sequenceExtra: sequenceExtra[id],
      hasAudio: hasAudio[id] ?? false,
      // Until the probe has answered, a layer is assumed to have a picture:
      // the visibility switch is the one every layer but a music track uses,
      // so appearing and then going is far less startling than the reverse.
      hasPicture: hasPicture[id] ?? true,
      groupHeader: groupHeaders[id],
    ));
  }
  return out;
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

/// Which slot footage dropped from the Project panel takes, from how far down
/// the stack it landed.
///
/// [y] is measured from the top of the first block, so the caller subtracts the
/// pinned toolbar and header and adds the scroll. A drop in a block's top half
/// goes above it and one in its bottom half below, the same midpoint rule a
/// layer drag uses — and a drop past the last block lands at the bottom.
int layerDropSlot(List<double> heights, double y) {
  var top = 0.0;
  for (var i = 0; i < heights.length; i++) {
    if (y < top + heights[i] / 2) return i;
    top += heights[i];
  }
  return heights.length;
}

/// The rows one twirl opens or shuts (§6.4).
///
/// [path] alone, unless it is itself one of [selected] — in which case the
/// whole selection travels with it, every row taking the clicked row's new
/// state. Pure, so the rule is checked without a widget tree.
Set<String> rowsTwirledWith(String path, Set<String> selected) =>
    selected.contains(path) ? {path, ...selected} : {path};

/// One layer's block, slid out of a dragged layer's way.
///
/// A transform, not a layout change: the rows keep their places, so a drag
/// never reflows the table under itself — and the same widget wraps the block
/// in the outline and the block in the lanes, which is what keeps them
/// together to the pixel.
class LayerDragSlide extends StatelessWidget {
  final ValueListenable<LayerDrag?> drag;
  final List<double> heights;
  final int index;
  final Widget child;

  const LayerDragSlide({
    super.key,
    required this.drag,
    required this.heights,
    required this.index,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    // The user's animation level, not a constant: at *None* the rows must
    // arrive without travelling at all (15-DESIGN §8), and a hard-coded
    // duration here would be one animation the setting could not reach.
    final duration = animationDuration(ThemeScope.of(context).animationLevel);
    return ValueListenableBuilder<LayerDrag?>(
      valueListenable: drag,
      child: child,
      builder: (context, value, child) {
        final height = index < heights.length ? heights[index] : 0.0;
        return AnimatedSlide(
          offset: height <= 0
              ? Offset.zero
              : Offset(0, layerDragShift(heights, value, index) / height),
          duration: duration,
          curve: Curves.easeOut,
          child: child,
        );
      },
    );
  }
}

/// Which blocks a viewport [viewport] tall, scrolled to [offset], has to have
/// built — as `[first, last)` into [heights].
///
/// **Three screenfuls: the one in view and one either side of it**, which is
/// not a guess. Two things reach outside the visible band and both are bounded
/// by it — a layer drag slides the blocks it passes by the dragged block's own
/// height, and the dragged block itself travels no further than the pointer,
/// which is inside the viewport. Build a screenful either side and every block
/// a gesture can move is a real widget while it moves.
///
/// The band is **slid back onto the stack** at either end of the scroll rather
/// than left hanging off it, so a stack shorter than three screenfuls is built
/// whole and a long one is given the same room at its ends as in its middle.
///
/// A viewport of zero has not been measured yet — before the first layout, or
/// in a test that never gave the panel a size — and windows nothing away, since
/// hiding the whole table is a worse answer than building it.
(int, int) blockWindow(List<double> heights, double offset, double viewport) {
  if (viewport <= 0) return (0, heights.length);
  final span = viewport * 3;
  var content = 0.0;
  for (final h in heights) {
    content += h;
  }
  final slack = content - span;
  final top = slack <= 0 ? 0.0 : (offset - viewport).clamp(0.0, slack);
  final bottom = top + span;
  var first = heights.length;
  var last = 0;
  var y = 0.0;
  for (var i = 0; i < heights.length; i++) {
    final next = y + heights[i];
    if (next > top && y < bottom) {
      if (i < first) first = i;
      last = i + 1;
    }
    y = next;
  }
  return first < last ? (first, last) : (0, 0);
}

/// A stack of layer blocks with only the ones in view built, and the rest held
/// open by a blank above and a blank below.
///
/// **Why this rather than a `ListView`.** Both halves of the Timeline built
/// every layer's block in a `Column` inside a scroll view, so a select-all, a
/// twirl or a `U` on the owner's `songcutfull` precomp walked thousands of
/// widgets and a delete rebuilt 2330 — cost that grew with the layer count
/// rather than with what is on screen. A sliver list is the ordinary answer,
/// and it cannot be used here: the lane half draws its ground, its marquee,
/// its row seams and its key-block box as `Positioned.fill` overlays *in
/// content coordinates* over the same stack, and a viewport would take that
/// stack's coordinate space away from them. Two blanks keep the content
/// exactly the height it always was — which is also what keeps the two halves'
/// `maxScrollExtent` equal, the thing the scroll mirror rests on — while the
/// middle costs what is visible.
class LazyBlocks extends StatefulWidget {
  const LazyBlocks({
    super.key,
    required this.controller,
    required this.heights,
    required this.viewport,
    required this.builder,
  });

  /// The scroll this stack sits in. Listened to rather than rebuilt from
  /// above: only a scroll that brings a *different* block into view costs
  /// anything at all.
  final ScrollController controller;

  /// Every block's height, in order — [LayerRow.height], which both halves
  /// already hand down as `blockHeights`.
  final List<double> heights;

  /// The viewport's height, from the half's own `LayoutBuilder`.
  final double viewport;

  final Widget Function(BuildContext context, int index) builder;

  @override
  State<LazyBlocks> createState() => _LazyBlocksState();
}

class _LazyBlocksState extends State<LazyBlocks> {
  /// The last offset a single attached position reported. Held rather than
  /// read, because [positionOf] answers null for the frame a rebuild has two
  /// views on one controller — and taking that as zero would snap the window
  /// to the top of the stack and blank the rows being looked at.
  double _offset = 0;
  late (int, int) _window = _windowNow();

  /// The blocks already built for this widget's [LazyBlocks.builder], by index.
  ///
  /// **This is what makes a scroll incremental** (K-678,
  /// docs/impl/ui-performance.md §4.3). A slide used to hand the `Column` a
  /// freshly built widget for every block in the window, so bringing one new
  /// row in at the edge rebuilt three screenfuls — a ~75 ms build frame at the
  /// owner's window, 8.6 fps. Handing back the *same instance* for a block
  /// that has not changed makes `Element.updateChild` short-circuit it: no
  /// rebuild, and no layout either, since its render object is never dirtied
  /// and its constraints have not moved. A slide then costs the entering rows.
  final Map<int, Widget> _built = {};

  /// The block at [i], built once per builder.
  ///
  /// Keyed by index because a `Column`'s children are matched to their elements
  /// **by key** once the list slides: without one, every block lands on the
  /// element that held its neighbour, `canUpdate` says yes, and it rebuilds
  /// there — cache or no cache.
  Widget _blockAt(BuildContext context, int i) => _built[i] ??=
      KeyedSubtree(key: ValueKey<int>(i), child: widget.builder(context, i));

  (int, int) _windowNow() {
    _offset = positionOf(widget.controller)?.pixels ?? _offset;
    return blockWindow(widget.heights, _offset, widget.viewport);
  }

  @override
  void initState() {
    super.initState();
    widget.controller.addListener(_follow);
  }

  @override
  void didUpdateWidget(covariant LazyBlocks old) {
    super.didUpdateWidget(old);
    if (old.controller != widget.controller) {
      old.controller.removeListener(_follow);
      widget.controller.addListener(_follow);
    }
    // A new widget carries a new builder closure — over new rows, a new
    // selection, a new zoom — so every cached block is answering from the last
    // one. Dropped whole: which of them actually moved is the panel's question,
    // not this stack's, and a rebuild of the window is what a panel rebuild has
    // always cost.
    _built.clear();
    // An edit or a resize is rebuilding this anyway, and either can change
    // which blocks are in view, so take the window again rather than hold one
    // measured against heights that have gone.
    _window = _windowNow();
  }

  @override
  void dispose() {
    widget.controller.removeListener(_follow);
    super.dispose();
  }

  void _follow() {
    final next = _windowNow();
    if (next == _window) return;
    setState(() => _window = next);
  }

  @override
  Widget build(BuildContext context) {
    final (first, last) = _window;
    var above = 0.0;
    for (var i = 0; i < first && i < widget.heights.length; i++) {
      above += widget.heights[i];
    }
    var below = 0.0;
    for (var i = last; i < widget.heights.length; i++) {
      below += widget.heights[i];
    }
    // Blocks the window has scrolled past are let go, so a long sweep down a
    // 2,000-layer precomp holds three screenfuls of widgets rather than the
    // whole comp's.
    _built.removeWhere((i, _) => i < first || i >= last);
    // **The stack keeps its own layer** (K-626's pattern). Everything drawn
    // over the blocks — the playhead, the marquee, the work-area wash — sits
    // in the same `Stack` as they do, and a stack that is relaid out repaints
    // every child of it that has no layer of its own. That put the whole cost
    // of the blocks on screen behind a single vertical line moving, which is
    // exactly what a scrub over cached frames is. Behind a boundary, the
    // blocks are repainted when the blocks change and not when something above
    // them does.
    return RepaintBoundary(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          if (above > 0) SizedBox(height: above),
          for (var i = first; i < last; i++) _blockAt(context, i),
          if (below > 0) SizedBox(height: below),
        ],
      ),
    );
  }
}

/// A controller's scroll position, or null when there is not exactly one
/// view attached.
///
/// `ScrollController.offset` and `.position` both assert on a controller with
/// two views, which happens for a frame whenever a rebuild inserts the new
/// scroll view before the old one detaches — a drop target lighting up over
/// the panel was enough to hit it.
ScrollPosition? positionOf(ScrollController controller) =>
    controller.positions.length == 1 ? controller.positions.first : null;

/// The Timeline's two views (K-529, §12A.1), in the order their tabs sit.
///
/// Both share the ruler, the cache bar, the work area, the markers, the
/// playhead **and the outline** — what changes is the body under them: bars or
/// curves. Keys, the dope sheet, was the third and is gone.
enum TimelineMode { layers, graph }
