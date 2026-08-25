// The Viewer's layer controls: the wireframe boxes, the handles that scale and
// rotate, the click that selects, the drag that moves, and the marquee that
// selects several at once (K-217, docs/07 §2.3).
//
// **In plain terms.** Everything you can do to a layer with the mouse *on the
// picture* is here. A box is drawn round each selected layer, turned the way
// the layer is turned; eight small squares on its edges resize it; a short bar
// standing off its top rotates it; dragging inside it moves it; dragging from
// empty space rubber-bands a rectangle and takes everything wholly inside it.
// Hovering an unselected layer shows its box faintly, so a click never selects
// something you could not see coming.
//
// **What is geometry and what is a widget.** The arithmetic — where a handle
// sits, which layer a point is inside, whether a box is wholly within a
// rectangle, what scale a dragged handle implies — is plain functions at the
// top of this file, tested without a widget tree. [ViewerGizmoLayer] below is
// the part that listens to a pointer and commits ops; it holds no maths of its
// own beyond routing a gesture to one of those functions.
//
// **Whose transform is whose.** Every box is built from the comp read model
// (K-184), so drawing costs no bridge calls. Edits go through the layer's own
// reference handle, as everywhere else. A layer whose position is animated has
// no single point to drag, so it gets a box and no handles — the same rule the
// move handle had before this.

import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:uuid/uuid.dart';

import '../state/layer_bounds.dart' show shapeContentsRect;
import '../state/preview_throttle.dart';
import '../widgets/controls.dart';
import 'viewer_anchor.dart';
import 'viewer_layer_map.dart';
import 'viewer_shapes.dart' show bezierPath, stillHalfFeather;
import 'viewer_tool_cursor.dart' show paintAnchorMark, paintMarquee;
import 'layer_fold_frb.dart';

/// How big a scale handle is drawn, and how far from it a press still counts.
///
/// The handle is a dense-surface control by 15-DESIGN §7.2's reckoning — eight
/// of them on a box that can be small — so it draws at 9px and hit-tests at 32,
/// exactly as the Timeline's keyframes do.
const double gizmoHandleSize = 9;
const double gizmoHandleSlop = 32;

/// How far the rotation bar stands off the top of the box, in screen pixels.
const double gizmoRotateReach = 28;

/// How close a press has to be to the **anchor** handle to grab it, rather than
/// starting a move of the layer (K-221).
///
/// Much tighter than [gizmoHandleSlop], and deliberately: the anchor usually
/// sits in the middle of the box, which is also the easiest place to grab a
/// layer to move it. A generous slop there would turn every body drag into a
/// pan-behind — the pivot would slide and the layer would not, which reads as
/// the drag being broken. So the pivot has to be *aimed at*.
const double gizmoAnchorSlop = 16;

/// The eight scale handles and the rotation knob.
///
/// The scale handles are named for where they sit on the layer's own box, so
/// "topLeft" stays the top left of the *layer* however the layer is turned —
/// which is what makes a rotated box's handles keep dragging the edge you
/// grabbed rather than the one that happens to be uppermost on screen.
enum GizmoHandle {
  topLeft,
  top,
  topRight,
  right,
  bottomRight,
  bottom,
  bottomLeft,
  left,
  rotate,

  /// The anchor point itself (K-221): dragging it pans behind — the pivot moves
  /// and the picture stays put, exactly as the Anchor point tool does (K-220).
  /// It sits wherever the layer's anchor is, which is usually but not always
  /// the middle of the box.
  anchor;

  /// Where this handle sits on the unit box (0..1 in each axis). The rotation
  /// knob shares the top edge's midpoint and is pushed off the box in screen
  /// space, because its distance is a fixed number of *screen* pixels — it must
  /// stay grabbable however far the picture is zoomed out.
  (double, double) get unit => switch (this) {
        GizmoHandle.topLeft => (0, 0),
        GizmoHandle.top || GizmoHandle.rotate => (0.5, 0),
        GizmoHandle.topRight => (1, 0),
        GizmoHandle.right => (1, 0.5),
        GizmoHandle.bottomRight => (1, 1),
        GizmoHandle.bottom => (0.5, 1),
        GizmoHandle.bottomLeft => (0, 1),
        GizmoHandle.left => (0, 0.5),
        // Not a corner of the box at all — [LayerBox.handleAt] answers this one
        // from the layer's own anchor.
        GizmoHandle.anchor => (0.5, 0.5),
      };

  /// The eight that scale, in the order they are drawn.
  static const List<GizmoHandle> scaling = [
    GizmoHandle.topLeft,
    GizmoHandle.top,
    GizmoHandle.topRight,
    GizmoHandle.right,
    GizmoHandle.bottomRight,
    GizmoHandle.bottom,
    GizmoHandle.bottomLeft,
    GizmoHandle.left,
  ];
}

/// One layer as the Viewer sees it: its transform, how big its content is, and
/// everything that follows from those two.
///
/// Built per paint from the read model, so it is cheap and always current.
class LayerBox {
  final LayerReference layer;
  final UuidValue id;
  final ViewerLayerMap map;

  /// The content's size in layer pixels (see state/layer_bounds.dart).
  final Size bounds;

  /// Whether the layer's position is a single point rather than a curve. False
  /// means the box is drawn and nothing on it can be dragged: a keyframed
  /// position has no one value for a drag to add to.
  final bool draggable;

  /// The same for scale and rotation, which is what the handles write. A layer
  /// with a keyframed scale still moves; it just grows no handles.
  final bool scalable;

  /// The layer's masks (K-222), so the Viewer can outline them. Read from the
  /// model with everything else, so drawing them costs no bridge calls.
  final List<BridgeMask> masks;

  /// A shape layer's own art (K-237), for the same reason and from the same
  /// read. Empty for every other kind of layer.
  ///
  /// A mask and a shape item hold the *same* path type — `BezierPath`, in
  /// `lumit_core::mask` — which is why the points of both can be aimed at,
  /// swept up and dragged by one piece of code. What differs is only where the
  /// edit is written back to (K-224 for masks; `setShapeContents` here).
  final List<BridgeShapeItem> shapeContents;

  /// Where the art's bounding box starts, in the art's own coordinates
  /// (K-308).
  ///
  /// A shape layer's picture **is** that box, so the layer's pixel (0, 0) is
  /// this corner: a vertex at art (x, y) is drawn at layer pixel
  /// (x − artOrigin.dx, y − artOrigin.dy). Without the subtraction the points
  /// sat a whole bounding box away from the art they belong to — which is what
  /// [shapePoint] exists to stop anyone forgetting.
  final Offset artOrigin;

  /// The layer's rotation in degrees, as the document holds it.
  ///
  /// Carried rather than recovered from [map]'s sine and cosine, which only
  /// ever answer between -180 and 180: a layer wound round twice would snap
  /// back to its first turn the moment the knob was touched.
  final double rotationDegrees;

  const LayerBox({
    required this.layer,
    required this.id,
    required this.map,
    required this.bounds,
    required this.draggable,
    required this.scalable,
    required this.rotationDegrees,
    this.masks = const [],
    this.shapeContents = const [],
    this.artOrigin = Offset.zero,
  });

  /// A point of the layer's **art** on screen (K-308).
  ///
  /// A mask's vertex is already in layer pixels and goes straight through
  /// [map]; a shape item's is in the art's own coordinates, which sit a
  /// bounding box away. Everything that draws or aims at a shape point comes
  /// through here, so there is one place for that subtraction to live.
  Offset shapePoint(double x, double y) =>
      map.toScreen(x - artOrigin.dx, y - artOrigin.dy);

  /// The same box with some of its view-state replaced — what the in-flight
  /// gestures below are built from. Everything not named is carried, so a copy
  /// can never silently drop a field the way the hand-rolled rebuilds did: a
  /// shape layer's art vanished from the overlay during every scale, turn and
  /// pivot, because [shapeContents] and [artOrigin] were never copied.
  LayerBox copyWith({ViewerLayerMap? map, double? rotationDegrees}) => LayerBox(
        layer: layer,
        id: id,
        map: map ?? this.map,
        bounds: bounds,
        draggable: draggable,
        scalable: scalable,
        rotationDegrees: rotationDegrees ?? this.rotationDegrees,
        masks: masks,
        shapeContents: shapeContents,
        artOrigin: artOrigin,
      );

  /// The same box with the layer scaled to [sxPercent] / [syPercent] — the
  /// shape a scale in flight has, before it is committed (K-230). Negative is
  /// allowed and means what it says: the layer is turned over.
  LayerBox scaledTo(double sxPercent, double syPercent) =>
      copyWith(map: map.scaledTo(sxPercent, syPercent));

  /// The same box with the pivot moved and Position compensating — the shape a
  /// pan-behind in flight has (K-235). The box does not move; the anchor mark
  /// on it does, which is the whole of what panning behind looks like.
  LayerBox pivotedAt(Offset anchor, Offset position) =>
      copyWith(map: map.pivotedAt(anchor.dx, anchor.dy, position: position));

  /// The same box with the layer turned to [degrees] — the shape a rotation in
  /// flight has, before it is committed (K-230).
  LayerBox turnedTo(double degrees) =>
      copyWith(map: map.turnedTo(degrees), rotationDegrees: degrees);

  /// The box's four corners in screen space, clockwise from the layer's own
  /// top-left. A rotated layer therefore gives a rotated quad, not an
  /// axis-aligned rectangle — the box turns with the layer.
  List<Offset> get corners => [
        map.toScreen(0, 0),
        map.toScreen(bounds.width, 0),
        map.toScreen(bounds.width, bounds.height),
        map.toScreen(0, bounds.height),
      ];

  /// Where the layer's anchor — the point it scales and rotates about — is on
  /// screen.
  Offset get anchorScreen => map.toScreen(map.ax, map.ay);

  /// Whether [point] (screen space) is inside the layer's own rectangle.
  ///
  /// Answered in *layer* space rather than by a polygon test on screen, which
  /// is both simpler and exact under rotation, scale and pan: the inverse map
  /// already undoes all three.
  bool contains(Offset point) {
    final p = map.layerOf(point);
    return p.dx >= 0 &&
        p.dy >= 0 &&
        p.dx <= bounds.width &&
        p.dy <= bounds.height;
  }

  /// Whether the whole box lies within [rect] — the marquee's rule. Every
  /// corner must be inside, so a layer half-caught by the rubber band is not
  /// selected (After Effects' own behaviour, and the one that makes a sloppy
  /// sweep predictable).
  bool insideRect(Rect rect) => corners.every(rect.contains);

  /// Where [handle] is drawn, in screen space.
  Offset handleAt(GizmoHandle handle) {
    if (handle == GizmoHandle.anchor) return anchorScreen;
    final (ux, uy) = handle.unit;
    final point = map.toScreen(ux * bounds.width, uy * bounds.height);
    if (handle != GizmoHandle.rotate) return point;
    // The knob stands off the top edge along the box's own "up", so it turns
    // with the layer exactly as After Effects' does.
    final up = _up;
    return point + up * gizmoRotateReach;
  }

  /// The box's own upward direction on screen, as a unit vector: the top edge's
  /// midpoint minus the bottom edge's, normalised. Falls back to straight up
  /// for a degenerate (zero-height) box.
  Offset get _up {
    final top = map.toScreen(bounds.width / 2, 0);
    final bottom = map.toScreen(bounds.width / 2, bounds.height);
    final d = top - bottom;
    final len = d.distance;
    return len < 1e-6 ? const Offset(0, -1) : d / len;
  }

  /// The handle under [point], or null when none is. Nearest wins, so two
  /// handles whose slop overlaps on a small box do not fight.
  GizmoHandle? handleHit(Offset point) {
    GizmoHandle? best;
    var bestDistance = gizmoHandleSlop / 2;
    // The anchor first in the list only matters for ties; nearest still wins.
    for (final handle in [
      GizmoHandle.anchor,
      ...GizmoHandle.scaling,
      GizmoHandle.rotate,
    ]) {
      final slop = handle == GizmoHandle.anchor
          ? gizmoAnchorSlop / 2
          : gizmoHandleSlop / 2;
      final d = (handleAt(handle) - point).distance;
      if (d <= slop && d <= bestDistance) {
        bestDistance = d;
        best = handle;
      }
    }
    return best;
  }
}

/// One vertex of one mask, named so a selection can hold it: which layer, which
/// mask, and which point along it (K-224).
///
/// A plain string rather than a record, because it is a *set* key: two points
/// are the same point when their names match, and a string says that without a
/// hashCode to write.
String maskPointKey(UuidValue layerId, UuidValue maskId, int index) =>
    '$layerId#$maskId#$index';

/// The same for a shape item's vertex (K-237).
///
/// Prefixed, because a mask and a shape item are told apart by nothing else: a
/// layer could hold both, their ids are both UUIDs, and the two are written
/// back to the document by different calls. The prefix is what carries that
/// difference through a `Set<String>` of selected points.
String shapePointKey(UuidValue layerId, UuidValue itemId, int index) =>
    'shape#$layerId#$itemId#$index';

/// One editable vertex on the picture: where it is, what names it, and which of
/// the layer's paths it belongs to.
typedef PathPoint = ({
  String key,
  Offset at,
  UuidValue pathId,
  int index,
  bool shape,
});

/// Every vertex of [box] that can be aimed at — its masks' and, for a shape
/// layer, its own art's.
///
/// Masks and shape items hold the same path type, so this is one walk over two
/// lists rather than two kinds of point.
List<PathPoint> pathPointsOf(LayerBox box) {
  final out = <PathPoint>[];
  for (final mask in box.masks) {
    for (var i = 0; i < mask.vertices.length; i++) {
      final v = mask.vertices[i];
      out.add((
        key: maskPointKey(box.id, mask.id, i),
        at: box.map.toScreen(v.x, v.y),
        pathId: mask.id,
        index: i,
        shape: false,
      ));
    }
  }
  for (final item in box.shapeContents) {
    for (var i = 0; i < item.vertices.length; i++) {
      final v = item.vertices[i];
      out.add((
        key: shapePointKey(box.id, item.id, i),
        at: box.shapePoint(v.x, v.y),
        pathId: item.id,
        index: i,
        shape: true,
      ));
    }
  }
  return out;
}

/// The editable point under [point] across [boxes], or null when none is near
/// enough. Nearest wins, so two points close together do not fight.
({LayerBox box, String key, UuidValue pathId, int index, bool shape})?
    pathPointAt(
  List<LayerBox> boxes,
  Offset point, {
  double slop = gizmoAnchorSlop / 2,
}) {
  ({LayerBox box, String key, UuidValue pathId, int index, bool shape})? best;
  var bestDistance = slop;
  for (final box in boxes) {
    for (final p in pathPointsOf(box)) {
      final d = (p.at - point).distance;
      if (d <= bestDistance) {
        bestDistance = d;
        best = (
          box: box,
          key: p.key,
          pathId: p.pathId,
          index: p.index,
          shape: p.shape,
        );
      }
    }
  }
  return best;
}

/// Every editable point of [boxes] inside [rect] — what a marquee catches when
/// it is sweeping points rather than layers.
Set<String> pathPointsInRect(List<LayerBox> boxes, Rect rect) => {
      for (final box in boxes)
        for (final p in pathPointsOf(box))
          if (rect.contains(p.at)) p.key,
    };

/// A drag's screen delta in [box]'s own coordinates.
///
/// Two points on the picture, subtracted, so the layer's scale and rotation are
/// undone exactly — which is what lets a selection spanning two layers with
/// different transforms still move together on screen.
Offset pointDeltaIn(LayerBox box, Offset screenDelta) =>
    box.map.layerOf(screenDelta) - box.map.layerOf(Offset.zero);

List<BridgeVertex> _verticesMoved(
  List<BridgeVertex> vertices,
  bool Function(int index) moved,
  Offset d,
) =>
    [
      for (var i = 0; i < vertices.length; i++)
        if (moved(i))
          BridgeVertex(
            x: vertices[i].x + d.dx,
            y: vertices[i].y + d.dy,
            tanInX: vertices[i].tanInX,
            tanInY: vertices[i].tanInY,
            tanOutX: vertices[i].tanOutX,
            tanOutY: vertices[i].tanOutY,
          )
        else
          vertices[i],
    ];

/// [mask] with every point of it in [points] moved by [d] (layer coordinates),
/// or null when none of its points is selected.
///
/// The preview and the commit both come through here, so what the drag shows is
/// what the release writes.
BridgeMask? maskWithPointsMoved(
  LayerBox box,
  BridgeMask mask,
  Set<String> points,
  Offset d,
) {
  bool moved(int i) => points.contains(maskPointKey(box.id, mask.id, i));
  if (!Iterable<int>.generate(mask.vertices.length).any(moved)) return null;
  return BridgeMask(
    id: mask.id,
    name: mask.name,
    vertices: _verticesMoved(mask.vertices, moved, d),
    closed: mask.closed,
    inverted: mask.inverted,
    opacity: mask.opacity,
    mode: mask.mode,
    feather: mask.feather,
    vertexFeather: mask.vertexFeather,
    expansion: mask.expansion,
    pathKeys: mask.pathKeys,
  );
}

/// [box]'s whole art with every point of it in [points] moved by [d], or null
/// when none of it is selected.
///
/// The whole list, because that is how art is written back (`setShapeContents`,
/// K-283) — one op for a layer however many of its items a drag caught.
List<BridgeShapeItem>? shapeContentsWithPointsMoved(
  LayerBox box,
  Set<String> points,
  Offset d,
) {
  var touched = false;
  final contents = <BridgeShapeItem>[];
  for (final item in box.shapeContents) {
    bool moved(int i) => points.contains(shapePointKey(box.id, item.id, i));
    if (!Iterable<int>.generate(item.vertices.length).any(moved)) {
      contents.add(item);
      continue;
    }
    touched = true;
    // A point drag moves geometry; every modifier on the item is carried
    // over untouched.
    contents.add(shapeItemWith(item,
        vertices: _verticesMoved(item.vertices, moved, d)));
  }
  return touched ? contents : null;
}

/// Which layer a click at [point] lands on: the topmost whose box contains it.
///
/// [boxes] is in stacking order, top first — the order the read model reports
/// layers in — so the first hit is the one a user would say is "on top".
LayerBox? layerAtPoint(List<LayerBox> boxes, Offset point) {
  for (final box in boxes) {
    if (box.contains(point)) return box;
  }
  return null;
}

/// Which layer a *drag* at [point] picks up (K-230).
///
/// A press inside something already selected grabs **that**, even when a layer
/// higher in the stack overlaps the same spot. Without this rule a layer chosen
/// in the Timeline could not be dragged wherever anything covered it: the press
/// silently swapped the selection for whatever was on top and moved that
/// instead, which is the drag doing something the user never asked for.
///
/// A plain click still takes the topmost ([layerAtPoint]) — that is how a layer
/// underneath gets chosen with the mouse in the first place.
LayerBox? layerToDragAt(
  List<LayerBox> boxes,
  Offset point,
  Set<UuidValue> selectedIds,
) {
  for (final box in boxes) {
    if (selectedIds.contains(box.id) && box.contains(point)) return box;
  }
  return layerAtPoint(boxes, point);
}

/// Every layer wholly inside [rect] — what a released marquee selects.
List<LayerBox> layersInsideRect(List<LayerBox> boxes, Rect rect) => [
      for (final box in boxes)
        if (box.insideRect(rect)) box
    ];

/// The scale percentages a handle drag implies.
///
/// [uniform] is the Shift rule: both axes take the same factor, so the layer
/// keeps its proportions. The shared factor is the mean of what each axis asked
/// for, which is what makes a corner drag follow the pointer's diagonal rather
/// than snapping to whichever axis moved more.
///
/// An edge handle resolves only its own axis — the other has no offset from the
/// anchor to divide by — so under [uniform] the resolved axis drives both.
(double, double) scaleForGizmoHandle({
  required LayerBox box,
  required GizmoHandle handle,
  required Offset pointer,
  required bool uniform,
}) {
  final (ux, uy) = handle.unit;
  final map = box.map;
  final dx = ux * box.bounds.width - map.ax;
  final dy = uy * box.bounds.height - map.ay;
  final (sx, sy) = map.scaleForHandle(
    dxFromAnchor: dx,
    dyFromAnchor: dy,
    pointer: pointer,
  );
  if (!uniform) return (sx, sy);

  final currentX = map.sx * 100.0;
  final currentY = map.sy * 100.0;
  // Which axes this handle can speak for at all: one with no offset from the
  // anchor — an edge handle's other axis, or an anchor sitting on the edge
  // being dragged — has nothing to divide by and came back unchanged. Deciding
  // that from the *geometry* rather than from "did the number move?" is what
  // keeps a corner drag that happens to leave one axis where it was from being
  // read as an edge drag.
  final ratios = <double>[
    if (dx.abs() > 1e-9 && currentX.abs() > 1e-9) sx / currentX,
    if (dy.abs() > 1e-9 && currentY.abs() > 1e-9) sy / currentY,
  ];
  if (ratios.isEmpty) return (sx, sy);
  final factor = ratios.reduce((a, b) => a + b) / ratios.length;
  return (currentX * factor, currentY * factor);
}

/// The rotation, in degrees, that dragging the knob from [from] to [to] implies
/// about [anchor] — the angle swept, added to where the layer already was.
///
/// [uniform] is Shift again, snapping to 45° steps as After Effects does.
double rotationForDrag({
  required Offset anchor,
  required Offset from,
  required Offset to,
  required double current,
  required bool uniform,
}) {
  double angle(Offset p) => math.atan2(p.dy - anchor.dy, p.dx - anchor.dx);
  final swept = (angle(to) - angle(from)) * 180.0 / math.pi;
  final result = current + swept;
  if (!uniform) return result;
  return (result / 45.0).roundToDouble() * 45.0;
}

/// What the pointer is doing to the picture right now.
enum _GizmoDrag { none, move, scale, rotate, anchor, points, marquee }

/// The layer controls over the picture.
class ViewerGizmoLayer extends StatefulWidget {
  final CompositionReference comp;
  final LumitUiState uiState;

  /// Every layer of the fronted comp, top first, with its box.
  final List<LayerBox> boxes;

  /// Whether the boxes, handles and hover highlight are drawn at all — the
  /// Viewer bar's wireframe switch. Gestures still work when they are off:
  /// hiding the controls is about the *picture* being unobstructed, not about
  /// giving up the mouse (After Effects' Show Layer Controls is the same).
  final bool showControls;

  /// The armed tool. Selection edits; Hand only ever draws.
  final ToolMode tool;

  /// Whether each selected layer's anchor point is marked — the pin it turns
  /// on. Drawn while the Rotation tool is armed (K-219), where "about what?" is
  /// the question the picture has to answer.
  final bool showAnchors;

  final VoidCallback onChanged;

  const ViewerGizmoLayer({
    super.key,
    required this.comp,
    required this.uiState,
    required this.boxes,
    required this.showControls,
    required this.tool,
    required this.onChanged,
    this.showAnchors = false,
  });

  @override
  State<ViewerGizmoLayer> createState() => _ViewerGizmoLayerState();
}

class _ViewerGizmoLayerState extends State<ViewerGizmoLayer> {
  /// The pointer's own gesture, decided when a drag starts and held until it
  /// ends — so a drag that began on a handle keeps scaling even once the
  /// pointer has left the handle's slop.
  _GizmoDrag _drag = _GizmoDrag.none;

  /// The drag so far, in screen pixels (a move), or the pointer's current
  /// position (a scale, a rotation, a marquee).
  Offset _delta = Offset.zero;
  Offset _origin = Offset.zero;
  Offset _pointer = Offset.zero;

  GizmoHandle? _handle;

  /// Where the pointer went down.
  ///
  /// Not the same as where the drag *starts*: a pan is only recognised once the
  /// pointer has travelled the framework's slop, and `DragStartDetails` reports
  /// that later point. A handle is 9px across, so by then the press has left it
  /// and every handle drag was read as a drag of the layer's body. What the
  /// user grabbed is where they put the pointer down, so that is what the hit
  /// test uses.
  Offset? _downAt;

  /// The layer a scale or rotation is acting on, and its box as it was when the
  /// gesture started — the maths is all relative to that, so it must not be
  /// rebuilt from a document the drag is itself changing.
  LayerBox? _acting;

  /// The layer under the pointer, drawn faintly so a click is predictable.
  UuidValue? _hover;

  /// The mask points that are selected, by [maskPointKey] (K-224).
  ///
  /// Points, not layers: with a mask on the picture the same marquee that
  /// gathers layers gathers *vertices*, and a drag then moves them. Which of
  /// the two a gesture means is decided by what is under it — a press on a
  /// point edits the path, a press anywhere else is the layer's.
  final Set<String> _points = {};

  final PreviewThrottle _throttle = PreviewThrottle();

  @override
  void dispose() {
    _throttle.cancel();
    super.dispose();
  }

  bool get _selectionTool => widget.tool.group == ToolGroup.select;

  /// The boxes of the selected layers, in stacking order.
  List<LayerBox> get _selected {
    final ids = widget.uiState.selectedLayerIds;
    return [
      for (final box in widget.boxes)
        if (ids.contains(box.id)) box
    ];
  }

  /// The boxes to **outline**: the selected layers, plus any layer a picked
  /// property row belongs to (K-341).
  ///
  /// Picking a property is saying which layer is being worked on, so the
  /// picture should say so too — before this, keying a mask's opacity left the
  /// Viewer showing nothing at all, and there was no way to see which layer
  /// the curve on screen belonged to. Drawing only: what can be *dragged*
  /// stays the layer selection proper, so an outline never turns into a handle
  /// nobody asked for.
  List<LayerBox> get _outlined {
    final ids = widget.uiState.selectedLayerIds;
    final byProperty = <String>{
      for (final path in widget.uiState.selectedProperties.value)
        if (path.indexOf('/') > 0) path.substring(0, path.indexOf('/')),
    };
    if (byProperty.isEmpty) return _selected;
    return [
      for (final box in widget.boxes)
        if (ids.contains(box.id) || byProperty.contains(box.id.toString())) box
    ];
  }

  /// The boxes whose points may be aimed at: the selected layers, plus the
  /// layer owning a mask whose Path row is picked (K-341). Picking that row is
  /// saying "this is the shape I am editing", so its points become reachable
  /// without having to click the layer first.
  List<LayerBox> get _editablePointBoxes {
    final owner = _pathBeingEdited?.$1;
    if (owner == null) return _selected;
    final out = [..._selected];
    if (!out.any((b) => b.id == owner)) {
      for (final box in widget.boxes) {
        if (box.id == owner) out.add(box);
      }
    }
    return out;
  }

  /// The mask whose **Path** row is picked, if one is (K-341) — the shape the
  /// author is editing, so it is the one whose points are offered even when
  /// the layer itself was never clicked.
  (UuidValue, UuidValue)? get _pathBeingEdited {
    for (final path in widget.uiState.selectedProperties.value) {
      final parts = path.split('/');
      // <layer>/masks/<mask>/path
      if (parts.length == 4 &&
          parts[1] == 'masks' &&
          parts[3] == MaskValue.path.name) {
        try {
          return (
            UuidValue.fromString(parts[0]),
            UuidValue.fromString(parts[2])
          );
        } catch (_) {
          return null;
        }
      }
    }
    return null;
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final selected = [for (final box in _selected) _live(box)];
    final outlined = [for (final box in _outlined) _live(box)];
    // The single-selection case is the one that gets handles: scaling and
    // rotating a set about a shared box is a different gesture with its own
    // maths, and is not built (docs/TODO.md).
    final soleHandles = selected.length == 1 && selected.single.scalable
        ? selected.single
        : null;

    final painter = CustomPaint(
      painter: _GizmoPainter(
        selected: widget.showControls ? outlined : const [],
        hover: widget.showControls && _hover != null && _selectionTool
            ? _hoverBox()
            : null,
        handlesFor: widget.showControls && _selectionTool ? soleHandles : null,
        // The masks of what is selected: a mask you cannot see is a mask you
        // cannot judge, and until mask editing exists this outline is the only
        // sight of one on the picture (K-222).
        maskedBoxes: widget.showControls ? outlined : const [],
        selectedPoints: _points,
        pointNudge: _drag == _GizmoDrag.points ? _delta : Offset.zero,
        anchors: widget.showControls && widget.showAnchors
            ? [for (final box in selected) box.anchorScreen]
            : const [],
        marquee: _drag == _GizmoDrag.marquee ? _marqueeRect() : null,
        moved: _drag == _GizmoDrag.move ? _delta : Offset.zero,
        // `animated`, not `accent` (§3.1, K-466): the closed list gives that
        // colour to "this is selected or in hand", and **selected gizmo
        // handles** are named in it. The approved drawing agrees — the box
        // round the picture's selected layer and the mark on its anchor are
        // both drawn in the amber, not the clay — and §3.2 bans the accent
        // inside the Viewer's neutrality zone in the first place.
        accent: t.animated,
        hairline: t.hairlineStrong,
        surface: t.surface0,
      ),
    );

    // The Hand tool never edits: its boxes are a read-out, and the drag under
    // them belongs to the panel, which pans the picture with it.
    if (!_selectionTool) {
      return Positioned.fill(child: IgnorePointer(child: painter));
    }

    return Positioned.fill(
      child: MouseRegion(
        onHover: _onHover,
        onExit: (_) => _setHover(null),
        child: Listener(
          onPointerDown: (event) => _downAt = event.localPosition,
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTapUp: _onTapUp,
            onPanStart: _onPanStart,
            onPanUpdate: _onPanUpdate,
            onPanEnd: (_) => _onPanEnd(),
            onPanCancel: _onPanCancel,
            child: painter,
          ),
        ),
      ),
    );
  }

  /// [box] as the gesture in flight would have it (K-230).
  ///
  /// The picture underneath is previewed at the value being dragged towards
  /// while the document still holds the old one, so a box built from the
  /// document lags the picture and only catches up on release. Three gestures
  /// can be in flight, never at once: this gizmo's own rotation knob and its
  /// scale handles, both local, and the Rotation tool's turn, which arrives on
  /// [LumitUiState.liveRotations] from another layer of the Viewer's stack.
  LayerBox _live(LayerBox box) {
    if (_acting?.id == box.id) {
      switch (_drag) {
        case _GizmoDrag.rotate:
          final degrees = _rotationNow();
          if (degrees != null) return box.turnedTo(degrees);
        case _GizmoDrag.scale:
          final scale = _scaleNow();
          if (scale != null) return box.scaledTo(scale.$1, scale.$2);
        case _GizmoDrag.anchor:
          final anchor = _anchorNow();
          if (anchor != null) {
            return box.pivotedAt(anchor, panBehindFor(box, anchor));
          }
        default:
          break;
      }
    }
    final degrees = widget.uiState.liveRotations.value[box.id];
    return degrees == null ? box : box.turnedTo(degrees);
  }

  LayerBox? _hoverBox() {
    for (final box in widget.boxes) {
      if (box.id == _hover) return box;
    }
    return null;
  }

  Rect _marqueeRect() => Rect.fromPoints(_origin, _pointer);

  void _setHover(UuidValue? id) {
    if (_hover == id) return;
    setState(() => _hover = id);
  }

  /// Track what a click would select. Only the layers that are *not* already
  /// selected highlight: a box already drawn in the accent needs no second
  /// mark, and highlighting it would read as a second state.
  void _onHover(PointerHoverEvent event) {
    if (_drag != _GizmoDrag.none) return;
    final hit = layerAtPoint(widget.boxes, event.localPosition);
    final ids = widget.uiState.selectedLayerIds;
    _setHover(hit == null || ids.contains(hit.id) ? null : hit.id);
  }

  /// A click selects what is under it — with Shift, adds to or removes from the
  /// selection; on empty space, selects nothing.
  void _onTapUp(TapUpDetails details) {
    final shift = HardwareKeyboard.instance.isShiftPressed;
    // A mask point first: it sits on top of the layer it belongs to, and a
    // click on it means the point rather than the layer.
    final point = widget.showControls
        ? pathPointAt(_editablePointBoxes, details.localPosition)
        : null;
    if (point != null) {
      setState(() {
        if (!shift) {
          _points
            ..clear()
            ..add(point.key);
        } else if (!_points.remove(point.key)) {
          _points.add(point.key);
        }
      });
      return;
    }

    final hit = layerAtPoint(widget.boxes, details.localPosition);
    if (hit == null) {
      if (!shift) {
        setState(_points.clear);
        widget.uiState.clearSelection();
      }
      return;
    }
    if (shift) {
      widget.uiState.toggleSelected(hit.layer);
    } else {
      widget.uiState.setSelection([hit.layer]);
    }
    _setHover(null);
  }

  void _onPanStart(DragStartDetails details) {
    // The press, not the point the pan was recognised at (see [_downAt]).
    final at = _downAt ?? details.localPosition;
    _origin = at;
    _pointer = details.localPosition;
    // The travel already spent recognising the drag counts: without it a move
    // lags the pointer by the slop for the whole gesture.
    _delta = details.localPosition - at;
    _acting = null;
    _handle = null;

    // A handle first: it sits on the box's edge, where the layer's own body is
    // also a target, and the handle must win there.
    //
    // Except against a path point aimed at squarely (K-308). A shape layer's
    // box *is* its art's bounding box, so its outermost points sit exactly on
    // the corners the scale handles occupy — and a handle's reach is twice a
    // point's, so every corner of a drawn square was a scale and never an edit.
    // A press inside a point's own, tighter reach means the point.
    final selected = _selected;
    final aimedAt =
        widget.showControls ? pathPointAt(_editablePointBoxes, at) : null;
    if (aimedAt == null && selected.length == 1 && selected.single.scalable) {
      final handle = selected.single.handleHit(at);
      if (handle != null) {
        setState(() {
          _acting = selected.single;
          _handle = handle;
          _drag = switch (handle) {
            GizmoHandle.rotate => _GizmoDrag.rotate,
            GizmoHandle.anchor => _GizmoDrag.anchor,
            _ => _GizmoDrag.scale,
          };
        });
        return;
      }
    }

    // A path point, on a layer that is selected: dragging it edits the path.
    // Only on selected layers, because a stray point of some layer underneath
    // must not steal a press meant for the picture.
    final point = aimedAt;
    if (point != null) {
      setState(() {
        if (!_points.contains(point.key)) {
          if (!HardwareKeyboard.instance.isShiftPressed) _points.clear();
          _points.add(point.key);
        }
        _drag = _GizmoDrag.points;
      });
      return;
    }

    final hit =
        layerToDragAt(widget.boxes, at, widget.uiState.selectedLayerIds);
    if (hit == null) {
      // Empty picture: rubber-band. The selection is left alone until the band
      // is let go — partly so the boxes stay on screen while it is drawn, and
      // partly because a sweep over a *selected* layer's mask points gathers
      // points (K-224), which it could not do if the press had already dropped
      // the layer they belong to.
      setState(() => _drag = _GizmoDrag.marquee);
      return;
    }

    // Dragging a layer that is not selected selects it first — otherwise the
    // gesture would move something the user had not chosen. A layer already in
    // the selection leaves the selection alone, so a set can be dragged as one.
    if (!widget.uiState.selectedLayerIds.contains(hit.id)) {
      widget.uiState.setSelection([hit.layer]);
    }
    setState(() {
      _drag = _GizmoDrag.move;
      _hover = null;
    });
  }

  void _onPanUpdate(DragUpdateDetails details) {
    setState(() {
      _pointer = details.localPosition;
      _delta += details.delta;
    });
    switch (_drag) {
      case _GizmoDrag.move:
        _previewMove();
      case _GizmoDrag.scale:
        _previewScale();
      case _GizmoDrag.rotate:
        _previewRotate();
      case _GizmoDrag.anchor:
        _previewAnchor();
      case _GizmoDrag.points:
        _previewPoints();
      case _GizmoDrag.marquee || _GizmoDrag.none:
        break;
    }
  }

  void _onPanEnd() {
    switch (_drag) {
      case _GizmoDrag.move:
        _commitMove();
      case _GizmoDrag.scale:
        _commitScale();
      case _GizmoDrag.rotate:
        _commitRotate();
      case _GizmoDrag.anchor:
        _commitAnchor();
      case _GizmoDrag.points:
        _commitPoints();
      case _GizmoDrag.marquee:
        _commitMarquee();
      case _GizmoDrag.none:
        break;
    }
    _throttle.cancel();
    setState(() {
      _drag = _GizmoDrag.none;
      _delta = Offset.zero;
      _acting = null;
      _handle = null;
    });
  }

  void _onPanCancel() {
    _throttle.cancel();
    setState(() {
      _drag = _GizmoDrag.none;
      _delta = Offset.zero;
      _acting = null;
      _handle = null;
    });
  }

  // --- Move -----------------------------------------------------------------

  /// A live preview, but only for a single layer: the engine patches one
  /// layer's transform into a clone of the document per request (K-183's
  /// preview path), so a set being dragged shows the picture move on release
  /// instead. The boxes follow the pointer either way, which is what makes the
  /// gesture readable.
  void _previewMove() {
    final selected = _selected;
    if (selected.length != 1) return;
    _throttle.request(() => _sendMovePreview(selected.single));
  }

  void _sendMovePreview(LayerBox box) {
    final (x, y) = _movedPosition(box);
    _sendPreview(box, (tf) => transformWith(tf, positionX: x, positionY: y));
  }

  /// Ask for the provisional picture, and never let a refusal end the gesture.
  ///
  /// A preview is a courtesy: the drag is the user's, and it must finish and
  /// commit whatever the renderer is doing. Without this guard a machine with
  /// no working render worker — no GPU adapter, a worker that has stopped —
  /// threw out of the pointer handler and the drag died mid-stroke, taking the
  /// commit with it. The bridge throws on any refusal (docs/TODO.md: a panic or
  /// an error both arrive as a Dart throw), so the catch is deliberately broad.
  void _sendPreview(
      LayerBox box, BridgeTransform Function(BridgeTransform) patch) {
    try {
      widget.comp.renderFrameWithTransformPreview(
        frame: BigInt.from(widget.uiState.playheadFrame.value),
        scale: widget.uiState.viewerScale,
        layer: box.layer,
        transform: patch(box.layer.getTransform()),
      );
    } catch (_) {
      // The boxes still follow the pointer, and the commit still lands.
    }
  }

  (double, double) _movedPosition(LayerBox box) => (
        box.map.px + _delta.dx / box.map.viewScale,
        box.map.py + _delta.dy / box.map.viewScale,
      );

  void _commitMove() {
    if (_delta == Offset.zero) return;
    var landed = false;
    for (final box in _selected) {
      if (!box.draggable) continue;
      final (x, y) = _movedPosition(box);
      // One op for both axes (K-230). x and y are separate properties in the
      // model, and writing them separately made one drag cost two undo steps —
      // Ctrl+Z put the layer back half way, along one axis, which reads as the
      // undo being broken rather than as two honest edits.
      try {
        box.layer.setTransforms(
          props: const [
            BridgeTransformProp.positionX,
            BridgeTransformProp.positionY,
          ],
          values: [BridgeScalar.static_(x), BridgeScalar.static_(y)],
        );
        landed = true;
      } catch (_) {
        // A layer deleted while the drag was in flight. The rest still move.
      }
    }
    if (landed) widget.onChanged();
  }

  // --- Scale ----------------------------------------------------------------

  (double, double)? _scaleNow() {
    final box = _acting;
    final handle = _handle;
    if (box == null || handle == null) return null;
    return scaleForGizmoHandle(
      box: box,
      handle: handle,
      pointer: _pointer,
      uniform: HardwareKeyboard.instance.isShiftPressed,
    );
  }

  void _previewScale() {
    final box = _acting;
    final scale = _scaleNow();
    if (box == null || scale == null) return;
    _throttle.request(() => _sendPreview(
        box, (tf) => transformWith(tf, scaleX: scale.$1, scaleY: scale.$2)));
  }

  void _commitScale() {
    final box = _acting;
    final scale = _scaleNow();
    if (box == null || scale == null || _delta == Offset.zero) return;
    // One op for both axes, for the same reason a move is (K-230).
    box.layer.setTransforms(
      props: const [
        BridgeTransformProp.scaleX,
        BridgeTransformProp.scaleY,
      ],
      values: [
        BridgeScalar.static_(scale.$1),
        BridgeScalar.static_(scale.$2),
      ],
    );
    widget.onChanged();
  }

  // --- Rotate ---------------------------------------------------------------

  double? _rotationNow() {
    final box = _acting;
    if (box == null) return null;
    return rotationForDrag(
      anchor: box.anchorScreen,
      from: _origin,
      to: _pointer,
      current: box.rotationDegrees,
      uniform: HardwareKeyboard.instance.isShiftPressed,
    );
  }

  void _previewRotate() {
    final box = _acting;
    final rotation = _rotationNow();
    if (box == null || rotation == null) return;
    _throttle.request(
        () => _sendPreview(box, (tf) => transformWith(tf, rotation: rotation)));
  }

  void _commitRotate() {
    final box = _acting;
    final rotation = _rotationNow();
    if (box == null || rotation == null || _delta == Offset.zero) return;
    box.layer.setTransform(
        prop: BridgeTransformProp.rotation,
        value: BridgeScalar.static_(rotation));
    widget.onChanged();
  }

  // --- Anchor (pan behind) --------------------------------------------------

  /// Where the anchor is being dragged to, in layer space, with the same two
  /// modifiers the Anchor point tool has (K-220): Shift locks the drag to one
  /// screen axis, Ctrl/Cmd snaps to the layer's own key points. The rules are
  /// [wantedAnchorAt], shared with that tool, so the two cannot drift apart.
  Offset? _anchorNow() {
    final box = _acting;
    if (box == null) return null;
    final started = box.map.toScreen(box.map.ax, box.map.ay);
    return wantedAnchorAt(box, started + (_pointer - _origin),
        lockFrom: started);
  }

  void _previewAnchor() {
    final box = _acting;
    final anchor = _anchorNow();
    if (box == null || anchor == null) return;
    final position = panBehindFor(box, anchor);
    _throttle.request(() => _sendPreview(
          box,
          (tf) => transformWith(
            tf,
            anchorX: anchor.dx,
            anchorY: anchor.dy,
            positionX: position.dx,
            positionY: position.dy,
          ),
        ));
  }

  void _commitAnchor() {
    final box = _acting;
    final anchor = _anchorNow();
    if (box == null || anchor == null || _delta == Offset.zero) return;
    final position = panBehindFor(box, anchor);
    try {
      // One op for the four properties: half of this edit moves the picture,
      // which is the one thing panning behind promises not to do (K-220).
      box.layer.setTransforms(
        props: const [
          BridgeTransformProp.anchorX,
          BridgeTransformProp.anchorY,
          BridgeTransformProp.positionX,
          BridgeTransformProp.positionY,
        ],
        values: [
          BridgeScalar.static_(anchor.dx),
          BridgeScalar.static_(anchor.dy),
          BridgeScalar.static_(position.dx),
          BridgeScalar.static_(position.dy),
        ],
      );
      widget.onChanged();
    } catch (_) {
      // The layer went away mid-drag.
    }
  }

  // --- Mask points ----------------------------------------------------------

  /// Write every dragged point's new position (K-224).
  ///
  /// The drag is a screen delta; each point is moved in its **own layer's**
  /// space, so a selection spanning two layers with different transforms still
  /// moves together on screen. One `set_mask` per mask, which is one undo step
  /// per mask — the same rule the razor follows for a multi-layer cut.
  void _commitPoints() {
    if (_delta == Offset.zero || _points.isEmpty) return;
    var landed = false;
    for (final box in _selected) {
      final d = pointDeltaIn(box, _delta);
      for (final mask in box.masks) {
        final moved = maskWithPointsMoved(box, mask, _points, d);
        if (moved == null) continue;
        try {
          // The playhead goes with it: on a mask whose shape is keyed, the
          // drag belongs to the key sitting there rather than to the static
          // path, which `path_at` would ignore (K-340).
          box.layer.setMask(
            mask: moved,
            at: widget.comp
                .timeOfFrame(frame: widget.uiState.playheadFrame.value),
          );
          landed = true;
          // **A keyed shape edit shows itself in the Timeline** (K-341). The
          // drag has just written a keyframe; the row that keyframe belongs to
          // is the one the author now wants to see, and without this the key
          // lands on a row nobody is looking at.
          if (mask.pathKeys.isNotEmpty) {
            widget.uiState.requestSelectProperty(
              '${box.id}/masks/${mask.id}/${MaskValue.path.name}',
            );
          }
        } catch (_) {
          // The mask went away mid-drag; the rest still move.
        }
      }
      // A shape layer's own art moves the same way, and by the same maths — the
      // one difference is where it is written back. Masks are set one at a time
      // by id; shape contents are a whole list (`SetShapeContents`, K-283), so
      // a layer's items are rebuilt together and committed once. That is still
      // one undo step per layer, which is the rule K-224 set — and the engine
      // moves the layer with the art's box in that same op (K-308), so the
      // points nobody dragged stay where they are.
      final contents = shapeContentsWithPointsMoved(box, _points, d);
      if (contents != null) {
        try {
          box.layer.setShapeContents(contents: contents);
          landed = true;
        } catch (_) {
          // The layer or its art went away mid-drag; the rest still move.
        }
      }
    }
    if (landed) widget.onChanged();
  }

  /// The picture under a point drag, while it is being dragged (K-308).
  ///
  /// A point drag used to show only its wireframe and leave the picture until
  /// the release — the same "no room in the preview call" the transform drags
  /// once had, and the reason a path edit was a guess until you let go. The
  /// engine patches **one** layer's paths into a clone of the document per
  /// request, so this previews a single layer, exactly as a move does.
  void _previewPoints() {
    final touched = [
      for (final box in _selected)
        if (pathPointsOf(box).any((p) => _points.contains(p.key))) box,
    ];
    if (touched.length != 1) return;
    final box = touched.single;
    _throttle.request(() => _sendPointsPreview(box));
  }

  void _sendPointsPreview(LayerBox box) {
    final d = pointDeltaIn(box, _delta);
    final contents = shapeContentsWithPointsMoved(box, _points, d);
    try {
      if (contents != null) {
        // Art, and the layer moved with it: the layer's picture *is* the art's
        // bounding box, so a preview of the art alone would slide the untouched
        // half of it and the commit would slide it back.
        final art = shapeContentsRect(contents);
        final shift = art == null ? Offset.zero : art.topLeft - box.artOrigin;
        widget.comp.renderFrameWithShapePreview(
          frame: BigInt.from(widget.uiState.playheadFrame.value),
          scale: widget.uiState.viewerScale,
          layer: box.layer,
          contents: contents,
          transform: shift == Offset.zero
              ? null
              : transformWith(
                  box.layer.getTransform(),
                  positionX: box.map.px + shift.dx,
                  positionY: box.map.py + shift.dy,
                ),
        );
        return;
      }
      // ponytail: masks and art are previewed one or the other, art first — one
      // request patches one of them, and a layer whose mask *and* art are being
      // dragged at once is rare enough to catch up on release.
      final masks = [
        for (final mask in box.masks)
          maskWithPointsMoved(box, mask, _points, d) ?? mask,
      ];
      widget.comp.renderFrameWithMaskPreview(
        frame: BigInt.from(widget.uiState.playheadFrame.value),
        scale: widget.uiState.viewerScale,
        layer: box.layer,
        masks: masks,
      );
    } catch (_) {
      // A preview is a courtesy: the wireframe still follows the pointer and
      // the commit still lands.
    }
  }

  // --- Marquee --------------------------------------------------------------

  void _commitMarquee() {
    final rect = _marqueeRect();
    // A stray click that happened to be read as a tiny drag should not clear a
    // selection the user just made another way.
    if (rect.width < 3 && rect.height < 3) return;

    // A sweep over a selected layer's mask points gathers **points** (K-224):
    // with a path on screen that is what a rubber band means, and the layers
    // are already selected anyway. With none caught it is the layer sweep it
    // has always been.
    if (widget.showControls) {
      final caughtPoints = pathPointsInRect(_editablePointBoxes, rect);
      if (caughtPoints.isNotEmpty) {
        setState(() {
          if (!HardwareKeyboard.instance.isShiftPressed) _points.clear();
          _points.addAll(caughtPoints);
        });
        return;
      }
    }
    setState(_points.clear);
    final caught = layersInsideRect(widget.boxes, rect);
    final shift = HardwareKeyboard.instance.isShiftPressed;
    final layers = <LayerReference>[
      if (shift)
        for (final box in _selected) box.layer,
      for (final box in caught)
        if (!shift || !widget.uiState.selectedLayerIds.contains(box.id))
          box.layer,
    ];
    widget.uiState.setSelection(layers);
  }
}

/// [tf] with the named channels replaced by static values — the copy-with the
/// generated struct does not have, shared by every preview that patches a
/// transform (this gizmo, the Rotation tool and the Anchor point tool alike).
BridgeTransform transformWith(
  BridgeTransform tf, {
  double? anchorX,
  double? anchorY,
  double? positionX,
  double? positionY,
  double? scaleX,
  double? scaleY,
  double? rotation,
}) {
  BridgeScalar put(double? value, BridgeScalar keep) =>
      value == null ? keep : BridgeScalar.static_(value);
  return BridgeTransform(
    anchorX: put(anchorX, tf.anchorX),
    anchorY: put(anchorY, tf.anchorY),
    positionX: put(positionX, tf.positionX),
    positionY: put(positionY, tf.positionY),
    positionZ: tf.positionZ,
    scaleX: put(scaleX, tf.scaleX),
    scaleY: put(scaleY, tf.scaleY),
    rotation: put(rotation, tf.rotation),
    rotationX: tf.rotationX,
    rotationY: tf.rotationY,
    opacity: tf.opacity,
  );
}

/// Everything the gizmo draws: the selected boxes, the hovered one, the
/// handles, and the marquee.
class _GizmoPainter extends CustomPainter {
  final List<LayerBox> selected;

  /// The boxes whose masks are outlined.
  final List<LayerBox> maskedBoxes;

  /// The mask points that are selected, and how far a drag has moved them so
  /// far — so the path follows the pointer before the document hears about it.
  final Set<String> selectedPoints;
  final Offset pointNudge;
  final LayerBox? hover;
  final LayerBox? handlesFor;

  /// Where to mark an anchor point, in screen space.
  final List<Offset> anchors;
  final Rect? marquee;
  final Offset moved;

  /// The colour a selected box, its handles and its anchor mark are drawn in:
  /// `animated` (§3.1), which is the name the field keeps for its history.
  final Color accent;
  final Color hairline;
  final Color surface;

  const _GizmoPainter({
    required this.selected,
    required this.maskedBoxes,
    required this.selectedPoints,
    required this.pointNudge,
    required this.hover,
    required this.handlesFor,
    required this.anchors,
    required this.marquee,
    required this.moved,
    required this.accent,
    required this.hairline,
    required this.surface,
  });

  @override
  void paint(Canvas canvas, Size size) {
    // The layer a click would take: the same box, drawn faintly. Under the
    // selection, so a selected box is never dimmed by a hover on top of it.
    final hovered = hover;
    if (hovered != null) {
      _outline(canvas, hovered.corners, accent.withValues(alpha: 0.35));
    }

    for (final box in selected) {
      _outline(canvas, [for (final c in box.corners) c + moved], accent);
    }

    final handles = handlesFor;
    if (handles != null) {
      final corners = [for (final c in handles.corners) c + moved];
      // The rotation bar first, so the knob's outline draws over it.
      final top = (corners[0] + corners[1]) / 2;
      final knob = handles.handleAt(GizmoHandle.rotate) + moved;
      canvas.drawLine(
        top,
        knob,
        Paint()
          ..color = accent
          ..strokeWidth = 1,
      );
      _knob(canvas, knob);
      for (final handle in GizmoHandle.scaling) {
        _handle(canvas, handles.handleAt(handle) + moved);
      }
      // The pivot, which is now a handle in its own right (K-221): drawn as
      // the anchor's ring-and-cross rather than a square, so it never reads as
      // a ninth scale handle.
      paintAnchorMark(
          canvas, handles.handleAt(GizmoHandle.anchor) + moved, accent);
    }

    for (final box in maskedBoxes) {
      for (final mask in box.masks) {
        _pathOutline(
          canvas,
          box,
          mask.vertices,
          mask.closed,
          (i) => maskPointKey(box.id, mask.id, i),
          feather: stillHalfFeather(mask),
        );
      }
      // A shape layer's own art gets the same outline and the same vertices, so
      // drawn art can be seen point by point and corrected rather than redrawn
      // (K-237's "editing a shape layer's points on the picture").
      for (final item in box.shapeContents) {
        _pathOutline(
          canvas,
          box,
          item.vertices,
          item.closed,
          (i) => shapePointKey(box.id, item.id, i),
          // Art coordinates, a bounding box away from the layer's own (K-308).
          art: true,
        );
      }
    }

    for (final anchor in anchors) {
      paintAnchorMark(canvas, anchor + moved, accent);
    }

    final band = marquee;
    if (band != null) paintMarquee(canvas, band, accent);
  }

  void _outline(Canvas canvas, List<Offset> corners, Color colour) {
    canvas.drawPath(
      Path()..addPolygon(corners, true),
      Paint()
        ..color = colour
        ..strokeWidth = 1
        ..style = PaintingStyle.stroke,
    );
  }

  /// A scale handle: a filled square with a hairline edge, so it reads on both
  /// a bright and a dark picture.
  void _handle(Canvas canvas, Offset at) {
    final rect = Rect.fromCenter(
      center: at,
      width: gizmoHandleSize,
      height: gizmoHandleSize,
    );
    canvas.drawRect(rect, Paint()..color = surface);
    canvas.drawRect(
      rect,
      Paint()
        ..color = accent
        ..strokeWidth = 1
        ..style = PaintingStyle.stroke,
    );
  }

  /// One path over the picture, in the layer's own space put through its
  /// transform — so the outline sits on the pixels it belongs to however the
  /// layer is moved or turned.
  ///
  /// Serves a mask (K-224) and a shape layer's own art (K-237) alike: the two
  /// hold the same path type, and only the key that names a vertex — and so
  /// where an edit is written back — differs.
  void _pathOutline(
    Canvas canvas,
    LayerBox box,
    List<BridgeVertex> vertices,
    bool closed,
    String Function(int index) keyOf, {
    bool art = false,
    List<double>? feather,
  }) {
    if (vertices.length < 2) return;
    // A selected point follows the pointer while it is being dragged, so the
    // path bends live rather than jumping on release.
    Offset nudgeFor(int i) =>
        selectedPoints.contains(keyOf(i)) ? pointNudge : Offset.zero;
    Offset screen(double x, double y) =>
        art ? box.shapePoint(x, y) : box.map.toScreen(x, y);
    Offset at(int i) {
      final v = vertices[i];
      return screen(v.x, v.y) + moved + nudgeFor(i);
    }

    Offset out(int i) {
      final v = vertices[i];
      return screen(v.x + v.tanOutX, v.y + v.tanOutY) + moved + nudgeFor(i);
    }

    Offset into(int i) {
      final v = vertices[i];
      return screen(v.x + v.tanInX, v.y + v.tanInY) + moved + nudgeFor(i);
    }

    canvas.drawPath(
      bezierPath(
        count: vertices.length,
        at: at,
        tangentOut: out,
        tangentIn: into,
        closed: closed,
      ),
      Paint()
        ..color = accent
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );

    // **How wide the soft edge is, either side of the path** (K-545). A pair
    // of dimmer lines rather than a second solid one, because they are a guide
    // to a width and not a path anybody can grab: they say where the feather
    // reaches, which is the only way a feather that varies from point to point
    // can be seen before the frame is drawn.
    if (feather != null && vertices.length >= 2) {
      final perPx = (screen(1, 0) - screen(0, 0)).distance;
      final segments = closed ? vertices.length : vertices.length - 1;
      final inside = <Offset>[];
      final outside = <Offset>[];
      for (var i = 0; i < segments; i++) {
        final j = (i + 1) % vertices.length;
        final (p0, p1, p2, p3) = (at(i), out(i), into(j), at(j));
        const steps = 12;
        for (var step = 0; step <= steps; step++) {
          final t = step / steps;
          final u = 1 - t;
          final p = p0 * (u * u * u) +
              p1 * (3 * u * u * t) +
              p2 * (3 * u * t * t) +
              p3 * (t * t * t);
          // The cubic's own derivative gives the direction the edge runs in,
          // and the feather is measured square to it.
          final d = (p1 - p0) * (3 * u * u) +
              (p2 - p1) * (6 * u * t) +
              (p3 - p2) * (3 * t * t);
          if (d.distance < 1e-6) continue;
          final normal = Offset(-d.dy, d.dx) / d.distance;
          final half =
              (feather[i] + (feather[j] - feather[i]) * t) * perPx;
          inside.add(p - normal * half);
          outside.add(p + normal * half);
        }
      }
      final guide = Paint()
        ..color = accent.withValues(alpha: 0.35)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1;
      for (final band in [inside, outside]) {
        if (band.length < 2) continue;
        canvas.drawPath(Path()..addPolygon(band, closed), guide);
      }
    }

    // Every vertex, so a path can be seen point by point (K-224): hollow when
    // it is merely there, filled when it is selected — the same "outline means
    // available, fill means chosen" the keyframe diamonds use.
    for (var i = 0; i < vertices.length; i++) {
      final selected = selectedPoints.contains(keyOf(i));
      final rect = Rect.fromCenter(center: at(i), width: 6, height: 6);
      canvas.drawRect(rect, Paint()..color = selected ? accent : surface);
      if (!selected) {
        canvas.drawRect(
          rect,
          Paint()
            ..color = accent
            ..strokeWidth = 1
            ..style = PaintingStyle.stroke,
        );
      }
    }
  }

  void _knob(Canvas canvas, Offset at) {
    canvas.drawCircle(at, gizmoHandleSize / 2, Paint()..color = surface);
    canvas.drawCircle(
      at,
      gizmoHandleSize / 2,
      Paint()
        ..color = accent
        ..strokeWidth = 1
        ..style = PaintingStyle.stroke,
    );
  }

  @override
  bool shouldRepaint(_GizmoPainter old) => true;
}
