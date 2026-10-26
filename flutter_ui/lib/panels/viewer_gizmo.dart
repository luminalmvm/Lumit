// The Viewer's layer controls: the wireframe boxes, the handles that scale and
// rotate, the click that selects, the drag that moves, and the marquee that
// selects several at once (docs/07 §2.3).
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
// **Whose transform is whose.** Every box is built from the comp read model,
// so drawing costs no bridge calls. Edits go through the layer's own
// reference handle, as everywhere else. A layer whose position is animated has
// no single point to drag, so its body does not drag; instead its **motion
// path** is drawn over the picture (docs/07 §2.4) — the line the engine
// sampled, a dot per frame, a box per key — and dragging a key's box moves
// that key.

import 'dart:math' as math;
import 'dart:ui' show PointMode;

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/keymap.dart' show BridgeKeyContext;
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../state/dock.dart' show Panel;
import '../state/layer_bounds.dart' show shapeContentsRect;
import '../state/preview_throttle.dart';
import '../widgets/controls.dart';
import '../widgets/share_marks.dart';
import 'timeline_extras_frb.dart' show DoubleTap, showMenuAt;
import 'timeline_snap.dart' show snapSuspended;
import 'viewer_anchor.dart';
import 'viewer_gizmo_3d.dart';
import 'viewer_layer_map.dart';
import 'viewer_overlays.dart'
    show ViewerOverlayPainter, viewerOverlayToggles, viewerToggleRow;
import 'viewer_shapes.dart' show bezierPath, stillHalfFeather;
import 'viewer_snap.dart' show snapViewerDrag, viewerSnapTargets;
import 'viewer_tool_cursor.dart' show paintAnchorMark, paintMarquee;
import 'viewer_type.dart' show viewerTypeEditRequest;
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
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
/// starting a move of the layer.
///
/// Much tighter than [gizmoHandleSlop], and deliberately: the anchor usually
/// sits in the middle of the box, which is also the easiest place to grab a
/// layer to move it. A generous slop there would turn every body drag into a
/// pan-behind — the pivot would slide and the layer would not, which reads as
/// the drag being broken. So the pivot has to be *aimed at*.
const double gizmoAnchorSlop = 16;

/// How far a light's mark reaches either side of its place, in screen pixels.
const double gizmoMarkerReach = 7;

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

  /// The anchor point itself: dragging it pans behind — the pivot moves
  /// and the picture stays put, exactly as the Anchor point tool does.
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

  /// The layer's masks, so the Viewer can outline them. Read from the
  /// model with everything else, so drawing them costs no bridge calls.
  final List<BridgeMask> masks;

  /// A shape layer's own art, for the same reason and from the same
  /// read. Empty for every other kind of layer.
  ///
  /// A mask and a shape item hold the *same* path type — `BezierPath`, in
  /// `lumit_core::mask` — which is why the points of both can be aimed at,
  /// swept up and dragged by one piece of code. What differs is only where the
  /// edit is written back to: masks one at a time, art via `setShapeContents`.
  final List<BridgeShapeItem> shapeContents;

  /// The shape item picked in the Timeline, or null. While one is picked its
  /// points are the only ones offered, so one piece of the art can be worked
  /// on without the rest getting in the way.
  final UuidValue? pickedItem;

  /// The items whose points are drawn and can be aimed at.
  Iterable<BridgeShapeItem> get offeredItems => pickedItem == null
      ? shapeContents
      : shapeContents.where((i) => i.id == pickedItem);

  /// Where the art's bounding box starts, in the art's own coordinates.
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

  /// The layer's puppet, or null on a layer nobody has pinned — which
  /// is most layers. Read from the model with everything else, so drawing the
  /// pins over the picture costs no bridge calls.
  final BridgePuppet? puppet;

  /// The layer's motion path, as the engine sampled it, or null on a layer
  /// whose Position is still or whose outline is not drawn. In comp pixels,
  /// because Position is where the anchor lands in the comp.
  final BridgeMotionPath? motionPath;

  /// Whether the layer is a point rather than a box: a light, which has a
  /// place in the scene and no picture. It is drawn as a small mark at its
  /// Position and picked by that.
  final bool marker;

  /// The transform the box was built from, which a 3D handle drag keys or
  /// sets, or null where nothing on the box writes it.
  final BridgeTransform? transform;

  /// A two-node camera's point of interest, which gets handles of its own.
  final Vec3? aim;

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
    this.pickedItem,
    this.artOrigin = Offset.zero,
    this.puppet,
    this.motionPath,
    this.marker = false,
    this.transform,
    this.aim,
  });

  /// A point of the layer's **art** on screen.
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
        pickedItem: pickedItem,
        artOrigin: artOrigin,
        puppet: puppet,
        motionPath: motionPath,
        marker: marker,
        transform: transform,
        aim: aim,
      );

  /// The same box with the layer scaled to [sxPercent] / [syPercent] — the
  /// shape a scale in flight has, before it is committed. Negative is
  /// allowed and means what it says: the layer is turned over.
  LayerBox scaledTo(double sxPercent, double syPercent) =>
      copyWith(map: map.scaledTo(sxPercent, syPercent));

  /// The same box with the pivot moved and Position compensating — the shape a
  /// pan-behind in flight has. The box does not move; the anchor mark
  /// on it does, which is the whole of what panning behind looks like.
  LayerBox pivotedAt(Offset anchor, Offset position, {double? z}) => copyWith(
      map: map.pivotedAt(anchor.dx, anchor.dy, position: position, z: z));

  /// The same box with the layer turned to [degrees] — the shape a rotation in
  /// flight has, before it is committed.
  LayerBox turnedTo(double degrees) =>
      copyWith(map: map.turnedTo(degrees), rotationDegrees: degrees);

  /// The box's four corners in screen space, clockwise from the layer's own
  /// top-left. A rotated layer therefore gives a rotated quad, not an
  /// axis-aligned rectangle — the box turns with the layer. Seen through a
  /// camera it is the four corners in perspective, and a marker's are a small
  /// diamond round its place.
  List<Offset> get corners {
    if (marker) {
      final at = anchorScreen;
      return [
        at + const Offset(0, -gizmoMarkerReach),
        at + const Offset(gizmoMarkerReach, 0),
        at + const Offset(0, gizmoMarkerReach),
        at + const Offset(-gizmoMarkerReach, 0),
      ];
    }
    return [
      map.toScreen(0, 0),
      map.toScreen(bounds.width, 0),
      map.toScreen(bounds.width, bounds.height),
      map.toScreen(0, bounds.height),
    ];
  }

  /// Whether every corner is in front of the eye, which a 2D layer always is.
  /// A box reaching behind it has no sensible place for its handles.
  bool get wholeInFront => marker
      ? map.inFront(map.ax, map.ay)
      : map.inFront(0, 0) &&
          map.inFront(bounds.width, 0) &&
          map.inFront(bounds.width, bounds.height) &&
          map.inFront(0, bounds.height);

  /// The outline drawn round the layer: [corners], less any part behind the
  /// eye, which a camera cannot see and has no screen point for.
  List<Offset> get outline {
    if (marker) return wholeInFront ? corners : const [];
    if (!map.projected) return corners;
    final quad = [
      (0.0, 0.0),
      (bounds.width, 0.0),
      (bounds.width, bounds.height),
      (0.0, bounds.height),
    ];
    // Cut on the layer itself, a hair in front of the eye.
    final kept = <(double, double)>[];
    for (var i = 0; i < quad.length; i++) {
      final a = quad[i];
      final b = quad[(i + 1) % quad.length];
      final da = map.depthOf(a.$1, a.$2) - viewerNear;
      final db = map.depthOf(b.$1, b.$2) - viewerNear;
      if (da >= 0) kept.add(a);
      if ((da >= 0) != (db >= 0)) {
        final t = da / (da - db);
        kept.add((a.$1 + (b.$1 - a.$1) * t, a.$2 + (b.$2 - a.$2) * t));
      }
    }
    return [for (final p in kept) map.toScreen(p.$1, p.$2)];
  }

  /// Where the layer's anchor — the point it scales and rotates about — is on
  /// screen.
  Offset get anchorScreen => map.toScreen(map.ax, map.ay);

  /// Whether [point] (screen space) is inside the layer's own rectangle.
  ///
  /// Answered in *layer* space rather than by a polygon test on screen, which
  /// is both simpler and exact under rotation, scale and pan: the inverse map
  /// already undoes all three. Seen through a camera the layer has to be in
  /// front of the eye there too. A marker is hit near its place.
  bool contains(Offset point) {
    if (marker) {
      return wholeInFront &&
          (anchorScreen - point).distance <= gizmoAnchorSlop / 2;
    }
    if (map.depthAt(point) == null) return false;
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
  bool insideRect(Rect rect) => wholeInFront && corners.every(rect.contains);

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
    if (marker || !wholeInFront) return null;
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

/// The box a drawing tool works on: the first of [boxes] — which are top first
/// — whose layer is in [selectedIds].
///
/// Every tool that draws onto a layer rather than at the composition needs the
/// same answer, and "the primary selection" has to mean the same thing in all
/// of them: the topmost selected layer, not whichever the selection set happens
/// to iterate first.
LayerBox? primarySelectedBox(List<LayerBox> boxes, Set<UuidValue> selectedIds) {
  for (final box in boxes) {
    if (selectedIds.contains(box.id)) return box;
  }
  return null;
}

/// One vertex of one mask, named so a selection can hold it: which layer, which
/// mask, and which point along it.
///
/// A plain string rather than a record, because it is a *set* key: two points
/// are the same point when their names match, and a string says that without a
/// hashCode to write.
String maskPointKey(UuidValue layerId, UuidValue maskId, int index) =>
    '$layerId#$maskId#$index';

/// The same for a shape item's vertex.
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
///
/// With [only], the layer and path whose Path row is picked, that layer offers
/// that path's points alone, so a layer of many paths shows just the one being
/// edited.
List<PathPoint> pathPointsOf(LayerBox box, {(UuidValue, UuidValue)? only}) {
  final out = <PathPoint>[];
  bool shown(UuidValue path) => showsPath(box, path, only);
  for (final mask in box.masks) {
    if (!shown(mask.id)) continue;
    for (var i = 0; i < mask.vertices.length; i++) {
      final v = mask.vertices[i];
      // Behind the eye a point is not drawn, so it can't be aimed at.
      if (!box.map.inFront(v.x, v.y)) continue;
      out.add((
        key: maskPointKey(box.id, mask.id, i),
        at: box.map.toScreen(v.x, v.y),
        pathId: mask.id,
        index: i,
        shape: false,
      ));
    }
  }
  for (final item in box.offeredItems) {
    if (!shown(item.id)) continue;
    for (var i = 0; i < item.vertices.length; i++) {
      final v = item.vertices[i];
      if (!box.map.inFront(v.x - box.artOrigin.dx, v.y - box.artOrigin.dy)) {
        continue;
      }
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
  (UuidValue, UuidValue)? only,
}) {
  ({LayerBox box, String key, UuidValue pathId, int index, bool shape})? best;
  var bestDistance = slop;
  for (final box in boxes) {
    for (final p in pathPointsOf(box, only: only)) {
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
Set<String> pathPointsInRect(List<LayerBox> boxes, Rect rect,
        {(UuidValue, UuidValue)? only}) =>
    {
      for (final box in boxes)
        for (final p in pathPointsOf(box, only: only))
          if (rect.contains(p.at)) p.key,
    };

/// Whether [box]'s path [path] is drawn and offered: always, unless [only]
/// names another path of the same layer.
bool showsPath(LayerBox box, UuidValue path, (UuidValue, UuidValue)? only) =>
    only == null || only.$1 != box.id || only.$2 == path;

/// One path of a layer as a list of vertices, whichever kind it is, with what
/// it takes to name its points and put them on screen.
typedef PathOnBox = ({
  UuidValue id,
  List<BridgeVertex> vertices,
  bool closed,
  bool shape,
  String Function(int index) keyOf,
  Offset Function(double x, double y) screen,
});

/// Every path of [box], masks first then the art, as [pathPointsOf] walks them.
List<PathOnBox> pathsOf(LayerBox box, {(UuidValue, UuidValue)? only}) => [
      for (final mask in box.masks)
        if (showsPath(box, mask.id, only))
          (
            id: mask.id,
            vertices: mask.vertices,
            closed: mask.closed,
            shape: false,
            keyOf: (int i) => maskPointKey(box.id, mask.id, i),
            screen: box.map.toScreen,
          ),
      for (final item in box.shapeContents)
        if (showsPath(box, item.id, only))
          (
            id: item.id,
            vertices: item.vertices,
            closed: item.closed,
            shape: true,
            keyOf: (int i) => shapePointKey(box.id, item.id, i),
            screen: box.shapePoint,
          ),
    ];

/// Which of a point's two handles: the one the path arrives along, or the one
/// it leaves along.
enum HandleSide { arriving, leaving }

/// A handle of a selected point, under the pointer.
typedef PathHandle = ({
  LayerBox box,
  String key,
  int index,
  HandleSide side,
});

/// A vertex's handle on [side], in its own path's coordinates.
Offset handleOf(BridgeVertex v, HandleSide side) => side == HandleSide.arriving
    ? Offset(v.tanInX, v.tanInY)
    : Offset(v.tanOutX, v.tanOutY);

/// The handle of a **selected** point under [point], or null. Only selected
/// points show their handles, so only theirs can be grabbed. A handle folded
/// into its point has no end of its own and is left to the point.
PathHandle? pathHandleAt(
  List<LayerBox> boxes,
  Offset point,
  Set<String> selected, {
  double slop = gizmoAnchorSlop / 2,
  (UuidValue, UuidValue)? only,
}) {
  PathHandle? best;
  var bestDistance = slop;
  for (final box in boxes) {
    for (final path in pathsOf(box, only: only)) {
      for (var i = 0; i < path.vertices.length; i++) {
        final key = path.keyOf(i);
        if (!selected.contains(key)) continue;
        final v = path.vertices[i];
        for (final side in HandleSide.values) {
          final h = handleOf(v, side);
          if (h.distance < 1e-9) continue;
          final d = (path.screen(v.x + h.dx, v.y + h.dy) - point).distance;
          if (d <= bestDistance) {
            bestDistance = d;
            best = (box: box, key: key, index: i, side: side);
          }
        }
      }
    }
  }
  return best;
}

/// Whether [v] is a smooth point: two handles in one line, pointing apart.
/// Dragging one of a smooth point's handles turns the other with it.
bool isSmoothVertex(BridgeVertex v) {
  final a = handleOf(v, HandleSide.arriving);
  final b = handleOf(v, HandleSide.leaving);
  if (a.distance < 1e-9 || b.distance < 1e-9) return false;
  final cos = (a.dx * b.dx + a.dy * b.dy) / (a.distance * b.distance);
  return cos < -0.995;
}

BridgeVertex _vertexWith(BridgeVertex v, {Offset? arriving, Offset? leaving}) =>
    BridgeVertex(
      x: v.x,
      y: v.y,
      tanInX: arriving?.dx ?? v.tanInX,
      tanInY: arriving?.dy ?? v.tanInY,
      tanOutX: leaving?.dx ?? v.tanOutX,
      tanOutY: leaving?.dy ?? v.tanOutY,
    );

/// How a handle drag treats the point's other handle.
enum HandleLink {
  /// Left where it is: the point becomes, or stays, a corner.
  broken,

  /// Turned to stay in line, at its own length: a smooth point stays smooth.
  turned,

  /// The mirror of the dragged one, length and all: handles pulled out of a
  /// point.
  mirrored,
}

/// [vertices] with point [index]'s handle on [side] set to [handle].
List<BridgeVertex> verticesWithHandle(
  List<BridgeVertex> vertices,
  int index,
  HandleSide side,
  Offset handle,
  HandleLink link,
) {
  final v = vertices[index];
  final otherSide =
      side == HandleSide.arriving ? HandleSide.leaving : HandleSide.arriving;
  final other = handleOf(v, otherSide);
  final Offset? turned = switch (link) {
    HandleLink.broken => null,
    HandleLink.mirrored => -handle,
    HandleLink.turned => handle.distance < 1e-9
        ? null
        : -handle / handle.distance * other.distance,
  };
  final (arriving, leaving) =
      side == HandleSide.arriving ? (handle, turned) : (turned, handle);
  return [
    for (var i = 0; i < vertices.length; i++)
      if (i == index)
        _vertexWith(v, arriving: arriving, leaving: leaving)
      else
        vertices[i],
  ];
}

/// [vertices] with point [index] turned from smooth to a corner, or from a
/// corner to smooth: a point with any handle loses both, and a point with
/// none grows a pair along the line between its neighbours, each a third of
/// the way to its own neighbour, as After Effects' Convert Vertex does.
List<BridgeVertex> verticesConverted(
  List<BridgeVertex> vertices,
  int index,
  bool closed,
) {
  final v = vertices[index];
  final n = vertices.length;
  final hasHandles = handleOf(v, HandleSide.arriving).distance > 1e-9 ||
      handleOf(v, HandleSide.leaving).distance > 1e-9;
  BridgeVertex converted;
  if (hasHandles) {
    converted = _vertexWith(v, arriving: Offset.zero, leaving: Offset.zero);
  } else {
    final here = Offset(v.x, v.y);
    final prev = index > 0 || closed ? vertices[(index - 1 + n) % n] : null;
    final next = index < n - 1 || closed ? vertices[(index + 1) % n] : null;
    final p = prev == null ? here : Offset(prev.x, prev.y);
    final q = next == null ? here : Offset(next.x, next.y);
    final run = q - p;
    if (run.distance < 1e-9) return vertices;
    final dir = run / run.distance;
    converted = _vertexWith(
      v,
      arriving: -dir * ((here - p).distance / 3),
      leaving: dir * ((q - here).distance / 3),
    );
  }
  return [
    for (var i = 0; i < n; i++) i == index ? converted : vertices[i],
  ];
}

/// [vertices] without point [index], or null when the path would be left with
/// too few points to be a path.
List<BridgeVertex>? verticesWithoutPoint(
  List<BridgeVertex> vertices,
  int index,
  bool closed,
) {
  if (vertices.length <= (closed ? 3 : 2)) return null;
  return [
    for (var i = 0; i < vertices.length; i++)
      if (i != index) vertices[i],
  ];
}

/// [vertices] with a point added at [t] along the edge leaving point
/// [segment]. The edge is split where it lies, handles and all, so the path's
/// shape does not change.
List<BridgeVertex> verticesWithPointAdded(
  List<BridgeVertex> vertices,
  int segment,
  double t,
) {
  final n = vertices.length;
  final a = vertices[segment];
  final b = vertices[(segment + 1) % n];
  final p0 = Offset(a.x, a.y);
  final p1 = p0 + handleOf(a, HandleSide.leaving);
  final p3 = Offset(b.x, b.y);
  final p2 = p3 + handleOf(b, HandleSide.arriving);
  // De Casteljau: the two halves of the cubic, and the point between them.
  Offset lerp(Offset u, Offset w) => u + (w - u) * t;
  final q0 = lerp(p0, p1), q1 = lerp(p1, p2), q2 = lerp(p2, p3);
  final r0 = lerp(q0, q1), r1 = lerp(q1, q2);
  final s = lerp(r0, r1);
  final added = BridgeVertex(
    x: s.dx,
    y: s.dy,
    tanInX: r0.dx - s.dx,
    tanInY: r0.dy - s.dy,
    tanOutX: r1.dx - s.dx,
    tanOutY: r1.dy - s.dy,
  );
  final out = <BridgeVertex>[];
  for (var i = 0; i < n; i++) {
    if (i == segment) {
      out.add(_vertexWith(a, leaving: q0 - p0));
      out.add(added);
    } else if (i == (segment + 1) % n) {
      out.add(_vertexWith(b, arriving: q2 - p3));
    } else {
      out.add(vertices[i]);
    }
  }
  return out;
}

/// The edge of a path under [point], and how far along it, or null when no
/// edge is near enough. Each edge is walked on the layer and put on screen a
/// step at a time, so a layer seen through a camera is aimed at as drawn.
({LayerBox box, PathOnBox path, int segment, double t})? pathEdgeAt(
  List<LayerBox> boxes,
  Offset point, {
  double slop = gizmoAnchorSlop / 2,
  (UuidValue, UuidValue)? only,
}) {
  ({LayerBox box, PathOnBox path, int segment, double t})? best;
  var bestDistance = slop;
  for (final box in boxes) {
    for (final path in pathsOf(box, only: only)) {
      final v = path.vertices;
      final edges = path.closed ? v.length : v.length - 1;
      for (var s = 0; s < edges; s++) {
        final a = v[s], b = v[(s + 1) % v.length];
        final p0 = Offset(a.x, a.y);
        final p1 = p0 + handleOf(a, HandleSide.leaving);
        final p3 = Offset(b.x, b.y);
        final p2 = p3 + handleOf(b, HandleSide.arriving);
        const steps = 48;
        for (var k = 1; k < steps; k++) {
          final t = k / steps;
          final u = 1 - t;
          final p = p0 * (u * u * u) +
              p1 * (3 * u * u * t) +
              p2 * (3 * u * t * t) +
              p3 * (t * t * t);
          final d = (path.screen(p.dx, p.dy) - point).distance;
          if (d <= bestDistance) {
            bestDistance = d;
            best = (box: box, path: path, segment: s, t: t);
          }
        }
      }
    }
  }
  return best;
}

/// A drag's screen delta in [box]'s own coordinates.
///
/// Two points on the picture, subtracted, so the layer's scale and rotation are
/// undone exactly — which is what lets a selection spanning two layers with
/// different transforms still move together on screen. Seen through a camera
/// a step depends on where it is taken, so it is taken from [from], the
/// press, which keeps the grabbed point under the pointer.
Offset pointDeltaIn(LayerBox box, Offset screenDelta, {Offset? from}) {
  final base = box.map.projected ? from ?? box.anchorScreen : Offset.zero;
  return box.map.layerOf(base + screenDelta) - box.map.layerOf(base);
}

Vec3 _flat(Offset step) => (step.dx, step.dy, 0);

/// An arrow key's nudge, as the change it makes to Position. A 2D layer
/// moves [by] on screen. A 3D layer takes a whole step of [by]'s length on
/// the Position axis that runs most nearly the arrow's way on screen, with
/// that axis's sign, however near or far the layer is: z where z runs that
/// way, as Up does in a Top view. Null when no axis runs anywhere near it.
Vec3? nudgeStep(LayerBox box, Offset by) {
  if (!box.map.projected) return _flat(box.map.positionStep(by));
  final m = box.map;
  if (!m.inFront(m.ax, m.ay)) return null;
  final z = m.depth!.z;
  final at = m.parentToScreen(m.px, m.py);
  final arrow = by / by.distance;
  Vec3? best;
  var nearest = 0.5;
  for (final (axis, run) in [
    ((1.0, 0.0, 0.0), m.parentToScreen(m.px + 1, m.py) - at),
    ((0.0, 1.0, 0.0), m.parentToScreen(m.px, m.py + 1) - at),
    ((0.0, 0.0, 1.0), m.parentToScreen(m.px, m.py, z + 1) - at),
  ]) {
    if (run.distance < 1e-9) continue;
    final along = (run.dx * arrow.dx + run.dy * arrow.dy) / run.distance;
    if (along.abs() > nearest) {
      nearest = along.abs();
      final k = along.sign * by.distance;
      best = (axis.$1 * k, axis.$2 * k, axis.$3 * k);
    }
  }
  return best;
}

/// A Position-space point on screen. A motion path is in the parent's pixels
/// (the comp's, with no parent) rather than layer pixels, because Position is
/// where the layer's anchor lands there, so it goes through none of the
/// layer's own scale or rotation. [z] is a 3D layer's depth there.
Offset compToScreen(LayerBox box, double x, double y, [double? z]) =>
    box.map.parentToScreen(x, y, z);

/// [box]'s motion path in its parent's space, one point per comp frame, with
/// a 3D layer's depth; empty when the layer has none.
List<(double, double, double?)> _motionPathSamples(LayerBox box) {
  final path = box.motionPath;
  if (path == null) return const [];
  final depths = path.depths;
  return [
    for (var i = 0; i + 1 < path.samples.length; i += 2)
      (
        path.samples[i],
        path.samples[i + 1],
        i ~/ 2 < depths.length ? depths[i ~/ 2] : null,
      ),
  ];
}

/// [box]'s motion path on screen, one point per comp frame in front of the
/// eye; empty when the layer has none.
List<Offset> motionPathScreen(LayerBox box) => [
      for (final (x, y, z) in _motionPathSamples(box))
        if (box.map.parentInFront(x, y, z)) compToScreen(box, x, y, z),
    ];

/// [box]'s motion path as lines on screen, cut where it passes behind the
/// eye: a path flying past the camera is drawn up to the eye and on again
/// after it.
List<List<Offset>> motionPathRuns(LayerBox box) {
  final samples = _motionPathSamples(box);
  final runs = <List<Offset>>[];
  var run = <Offset>[];
  for (var i = 0; i < samples.length; i++) {
    final (x, y, z) = samples[i];
    final da = box.map.parentDepthOf(x, y, z) - viewerNear;
    if (da >= 0) run.add(compToScreen(box, x, y, z));
    if (i + 1 == samples.length) break;
    final (nx, ny, nz) = samples[i + 1];
    final db = box.map.parentDepthOf(nx, ny, nz) - viewerNear;
    if ((da >= 0) == (db >= 0)) continue;
    final t = da / (da - db);
    final az = z ?? 0;
    run.add(compToScreen(box, x + (nx - x) * t, y + (ny - y) * t,
        z == null ? null : az + ((nz ?? az) - az) * t));
    if (da >= 0) {
      runs.add(run);
      run = <Offset>[];
    }
  }
  if (run.isNotEmpty) runs.add(run);
  return runs;
}

/// Where [key] of [box]'s motion path is on screen.
Offset motionKeyScreen(LayerBox box, BridgeMotionKey key) =>
    compToScreen(box, key.x, key.y, box.map.projected ? key.z : null);

/// Whether [key] is in front of the eye, where it can be drawn and grabbed.
bool _keyInFront(LayerBox box, BridgeMotionKey key) =>
    box.map.parentInFront(key.x, key.y, box.map.projected ? key.z : null);

/// The motion-path key under [point] across [boxes], or null when none is
/// near enough. Nearest wins, as it does for a path's points.
({LayerBox box, int index})? motionKeyAt(
  List<LayerBox> boxes,
  Offset point, {
  double slop = gizmoAnchorSlop / 2,
}) {
  ({LayerBox box, int index})? best;
  var bestDistance = slop;
  for (final box in boxes) {
    final keys = box.motionPath?.keys;
    if (keys == null) continue;
    for (var i = 0; i < keys.length; i++) {
      if (!_keyInFront(box, keys[i])) continue;
      final d = (motionKeyScreen(box, keys[i]) - point).distance;
      if (d <= bestDistance) {
        bestDistance = d;
        best = (box: box, index: i);
      }
    }
  }
  return best;
}

/// [tf] with the motion-path key [key] moved to [to] (comp pixels): the
/// keyframe each axis has at that time takes the new value, and an axis with
/// no key there is left as it is. The preview and the commit both come through
/// here, so the picture cannot land anywhere but where the dot did.
BridgeTransform transformWithMotionKey(
  BridgeTransform tf,
  BridgeMotionKey key,
  Offset to,
) {
  BridgeScalar moved(BridgeScalar scalar, int? index, double value) {
    if (index == null || scalar is! BridgeScalar_Keyframed) return scalar;
    final keys = scalar.field0;
    if (index >= keys.length) return scalar;
    return BridgeScalar.keyframed([
      for (var i = 0; i < keys.length; i++)
        if (i == index)
          BridgeKeyframe(
            time: keys[i].time,
            value: value,
            interpIn: keys[i].interpIn,
            interpOut: keys[i].interpOut,
          )
        else
          keys[i],
    ]);
  }

  return BridgeTransform(
    anchorX: tf.anchorX,
    anchorY: tf.anchorY,
    positionX: moved(tf.positionX, key.xIndex, to.dx),
    positionY: moved(tf.positionY, key.yIndex, to.dy),
    positionZ: tf.positionZ,
    scaleX: tf.scaleX,
    scaleY: tf.scaleY,
    rotation: tf.rotation,
    rotationX: tf.rotationX,
    rotationY: tf.rotationY,
    opacity: tf.opacity,
  );
}

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
) =>
    maskEdited(box, mask, _pointsMoved(points, d));

/// What an edit does to one path: its vertices afterwards, or null when the
/// edit does not touch that path. [keyOf] names the path's points, so an edit
/// can tell which of them are selected or in hand.
typedef VertexEdit = List<BridgeVertex>? Function(
  String Function(int index) keyOf,
  List<BridgeVertex> vertices,
  bool closed,
);

VertexEdit _pointsMoved(Set<String> points, Offset d) =>
    (keyOf, vertices, closed) {
      bool moved(int i) => points.contains(keyOf(i));
      if (!Iterable<int>.generate(vertices.length).any(moved)) return null;
      return _verticesMoved(vertices, moved, d);
    };

/// [mask] with [edit] made to its path, or null when the edit leaves it be.
BridgeMask? maskEdited(LayerBox box, BridgeMask mask, VertexEdit edit) {
  final vertices = edit(
      (i) => maskPointKey(box.id, mask.id, i), mask.vertices, mask.closed);
  if (vertices == null) return null;
  return maskWith(mask,
      vertices: vertices, vertexFeather: _featherFollowing(mask, vertices));
}

/// A copy of [mask] with its path or its per-point widths replaced.
BridgeMask maskWith(
  BridgeMask mask, {
  List<BridgeVertex>? vertices,
  List<BridgeScalar>? vertexFeather,
}) =>
    BridgeMask(
      id: mask.id,
      name: mask.name,
      vertices: vertices ?? mask.vertices,
      closed: mask.closed,
      inverted: mask.inverted,
      opacity: mask.opacity,
      mode: mask.mode,
      feather: mask.feather,
      vertexFeather: vertexFeather ?? mask.vertexFeather,
      expansion: mask.expansion,
      pathKeys: mask.pathKeys,
    );

/// [mask]'s per-point widths for the path [to]. A width belongs to its point
/// by position, so when points are added or removed each width follows its
/// point, found where it sits, and a new point takes the width before it.
List<BridgeScalar> _featherFollowing(BridgeMask mask, List<BridgeVertex> to) {
  final from = mask.vertices;
  final widths = mask.vertexFeather;
  if (widths.isEmpty || to.length == from.length) return widths;
  BridgeScalar widthAt(int i) => i < widths.length ? widths[i] : mask.feather;
  final out = <BridgeScalar>[];
  var j = 0;
  for (final v in to) {
    var k = j;
    while (k < from.length && (from[k].x != v.x || from[k].y != v.y)) {
      k++;
    }
    if (k < from.length) {
      out.add(widthAt(k));
      j = k + 1;
    } else {
      out.add(out.isEmpty ? widthAt(0) : out.last);
    }
  }
  return out;
}

/// The soft edge's width at [mask]'s point [index], when it is one number
/// rather than keyed.
double? stillFeatherAt(BridgeMask mask, int index) {
  final s = index < mask.vertexFeather.length
      ? mask.vertexFeather[index]
      : mask.feather;
  return s is BridgeScalar_Static ? s.field0 : null;
}

/// [mask] with point [index]'s soft edge set to [width]. A mask with one width
/// all the way round gets a width per point first, each the one it had.
BridgeMask maskWithFeatherAt(BridgeMask mask, int index, double width) {
  final n = mask.vertices.length;
  return maskWith(mask, vertexFeather: [
    for (var i = 0; i < n; i++)
      if (i == index)
        BridgeScalar.static_(width)
      else if (i < mask.vertexFeather.length)
        mask.vertexFeather[i]
      else
        mask.feather,
  ]);
}

/// Which way is square to [mask]'s path at point [index], in its layer: the
/// way a soft edge is measured.
Offset featherNormalAt(BridgeMask mask, int index) {
  final v = mask.vertices;
  final n = v.length;
  final here = v[index];
  var run = handleOf(here, HandleSide.leaving) - handleOf(here, HandleSide.arriving);
  if (run.distance < 1e-9) {
    final prev = v[(index - 1 + n) % n], next = v[(index + 1) % n];
    run = Offset(next.x - prev.x, next.y - prev.y);
  }
  if (run.distance < 1e-9) return const Offset(0, 1);
  return Offset(-run.dy, run.dx) / run.distance;
}

/// [box]'s whole art with every point of it in [points] moved by [d], or null
/// when none of it is selected.
///
/// The whole list, because that is how art is written back (`setShapeContents`):
/// one op for a layer however many of its items a drag caught.
List<BridgeShapeItem>? shapeContentsWithPointsMoved(
  LayerBox box,
  Set<String> points,
  Offset d,
) =>
    shapeContentsEdited(box, _pointsMoved(points, d));

/// [box]'s whole art with [edit] made to each path, or null when the edit
/// touches none of them.
List<BridgeShapeItem>? shapeContentsEdited(LayerBox box, VertexEdit edit) {
  var touched = false;
  final contents = <BridgeShapeItem>[];
  for (final item in box.shapeContents) {
    final vertices = edit((i) => shapePointKey(box.id, item.id, i),
        item.vertices, item.closed);
    if (vertices == null) {
      contents.add(item);
      continue;
    }
    touched = true;
    // An edit changes geometry; every modifier on the item is carried
    // over untouched.
    contents.add(shapeItemWith(item, vertices: vertices));
  }
  return touched ? contents : null;
}

/// The shape item of [layerId] that [selected] property paths pick, if any:
/// its own row or one of the rows under it (`<layer>/contents/<item>/...`).
UuidValue? pickedShapeItem(List<String> selected, UuidValue layerId) {
  for (final path in selected) {
    final parts = path.split('/');
    if (parts.length >= 3 &&
        parts[0] == layerId.toString() &&
        parts[1] == 'contents') {
      try {
        return UuidValue.fromString(parts[2]);
      } catch (_) {
        return null;
      }
    }
  }
  return null;
}

/// Which layer a click at [point] lands on: the topmost whose box contains it.
///
/// [boxes] is in stacking order, top first — the order the read model reports
/// layers in — so the first hit is the one a user would say is "on top".
///
/// 3D layers next to each other in the stack are one run, as After Effects
/// sorts them, and within a run the one nearest the eye under the pointer
/// wins. A 2D layer ends a run.
LayerBox? layerAtPoint(List<LayerBox> boxes, Offset point) {
  LayerBox? nearest;
  var nearestDepth = double.infinity;
  for (final box in boxes) {
    if (!box.map.projected) {
      if (nearest != null) return nearest;
      if (box.contains(point)) return box;
      continue;
    }
    if (!box.contains(point)) continue;
    // A light's mark is as far away as the light is.
    final depth = box.marker
        ? box.map.depthOf(box.map.ax, box.map.ay)
        : box.map.depthAt(point) ?? double.infinity;
    if (depth < nearestDepth) {
      nearest = box;
      nearestDepth = depth;
    }
  }
  return nearest;
}

/// Which layer a *drag* at [point] picks up.
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

/// One set of 3D handles and the three values its arrows write.
class Handles3d {
  final LayerBox box;
  final Gizmo3d gizmo;

  /// x, y and z: Position, or a two-node camera's point of interest.
  final List<BridgeTransformProp> props;

  const Handles3d(this.box, this.gizmo, this.props);

  /// Where the handles are, as the values the arrows write.
  Vec3 get at => gizmo.at;
}

/// The channel [prop] names in [tf], for those a 3D handle writes.
BridgeScalar? scalarOf(BridgeTransform tf, BridgeTransformProp prop) =>
    switch (prop) {
      BridgeTransformProp.positionX => tf.positionX,
      BridgeTransformProp.positionY => tf.positionY,
      BridgeTransformProp.positionZ => tf.positionZ,
      BridgeTransformProp.rotation => tf.rotation,
      BridgeTransformProp.rotationX => tf.rotationX,
      BridgeTransformProp.rotationY => tf.rotationY,
      BridgeTransformProp.poiX => tf.camera?.poiX,
      BridgeTransformProp.poiY => tf.camera?.poiY,
      BridgeTransformProp.poiZ => tf.camera?.poiZ,
      BridgeTransformProp.orientationX => tf.depth?.orientationX,
      BridgeTransformProp.orientationY => tf.depth?.orientationY,
      BridgeTransformProp.orientationZ => tf.depth?.orientationZ,
      _ => null,
    };

/// [box]'s own axes in its parent's space: the columns of its whole turn,
/// its Orientation and any aim outside its rotations included.
List<Vec3> localAxesOfBox(LayerBox box) {
  final depth = box.map.depth;
  final m = rowsTurn(turnOfBox(box), ae: depth?.aeOrder ?? false);
  final o = depth?.outside;
  final whole = o == null ? m : mul3(o, m);
  return [
    for (var c = 0; c < 3; c++) (whole[c], whole[3 + c], whole[6 + c]),
  ];
}

/// How [box] is turned, X, Y and Z Rotation in degrees.
Vec3 turnOfBox(LayerBox box) => (
      box.map.depth?.rotationX ?? 0,
      box.map.depth?.rotationY ?? 0,
      box.rotationDegrees,
    );

/// Those of [ids] with an ancestor also among them. They follow that
/// ancestor, so moving or turning them as well would do it twice.
Set<UuidValue> ridingAlong(Set<UuidValue> ids, List<BridgeLayerEntry> layers) {
  final parentOf = {
    for (final entry in layers) entry.layer.internallayerId: entry.info.parent
  };
  bool rides(UuidValue id) {
    final seen = {id};
    for (var up = parentOf[id]; up != null && seen.add(up); up = parentOf[up]) {
      if (ids.contains(up)) return true;
    }
    return false;
  }

  return {
    for (final id in ids)
      if (rides(id)) id
  };
}

/// The box the 3D handles sit on among [boxes]: the layer picked first, as
/// [LumitUiState.selectedLayer] has it, while that one is seen in depth,
/// otherwise the first that is.
LayerBox? primary3d(List<LayerBox> boxes, UuidValue? first) {
  final deep = [
    for (final box in boxes)
      if (box.map.projected) box
  ];
  for (final box in deep) {
    if (box.id == first) return box;
  }
  return deep.firstOrNull;
}

/// The move handles of a 3D layer, a light or a camera: one set at its
/// anchor, and a second at a two-node camera's point of interest. None where
/// the place is behind the eye or a value it writes is an expression's.
List<Handles3d> handles3dOf(LayerBox box, GizmoAxes mode) {
  final tf = box.transform;
  final map = box.map;
  if (tf == null || !map.projected) return const [];
  final local = localAxesOfBox(box);
  final out = <Handles3d>[];
  for (final (at, props) in [
    (
      (map.px, map.py, map.depth!.z),
      const [
        BridgeTransformProp.positionX,
        BridgeTransformProp.positionY,
        BridgeTransformProp.positionZ,
      ]
    ),
    if (box.aim case final aim?)
      (
        aim,
        const [
          BridgeTransformProp.poiX,
          BridgeTransformProp.poiY,
          BridgeTransformProp.poiZ,
        ]
      ),
  ]) {
    if (props.any((p) => switch (scalarOf(tf, p)) {
          null || BridgeScalar_Expression() => true,
          _ => false,
        })) {
      continue;
    }
    final gizmo = Gizmo3d.of(map, at: at, local: local, mode: mode);
    if (gizmo != null) out.add(Handles3d(box, gizmo, props));
  }
  return out;
}

/// What the pointer is doing to the picture right now.
enum _GizmoDrag {
  none,
  move,
  scale,
  rotate,
  anchor,
  points,

  /// A handle of a selected path point, or a pair pulled out of a point.
  handle,

  /// A mask point's soft edge, with the Mask feather tool.
  feather,
  marquee,

  /// A key of a motion path (docs/07 §2.4), dragged on the picture.
  motionKey,

  /// A 3D arrow or square.
  axis3d,
}

/// Where the right-click menu's Align puts a layer's box against the
/// composition's: an edge to an edge, or a middle to a middle. [x] and [y]
/// are that point across and down, null on the axis left alone.
enum _Align {
  left(0, null),
  centre(0.5, null),
  right(1, null),
  top(null, 0),
  middle(null, 0.5),
  bottom(null, 1);

  const _Align(this.x, this.y);
  final double? x;
  final double? y;

  String get title => switch (this) {
        _Align.left => l10n.viewerAlignLeft,
        _Align.centre => l10n.viewerAlignCentre,
        _Align.right => l10n.viewerAlignRight,
        _Align.top => l10n.viewerAlignTop,
        _Align.middle => l10n.viewerAlignMiddle,
        _Align.bottom => l10n.viewerAlignBottom,
      };
}

/// The layer controls over the picture.
class ViewerGizmoLayer extends StatefulWidget {
  final CompositionReference comp;
  final LumitUiState uiState;

  /// Every layer of the fronted comp, top first, with its box.
  final List<LayerBox> boxes;

  /// The selected cameras, as marks at their eyes: not on the picture to
  /// click, but moved by their 3D handles once picked in the Timeline.
  final List<LayerBox> cameras;

  /// Whether the boxes, handles and hover highlight are drawn at all — the
  /// Viewer bar's wireframe switch. Gestures still work when they are off:
  /// hiding the controls is about the *picture* being unobstructed, not about
  /// giving up the mouse (After Effects' Show Layer Controls is the same).
  final bool showControls;

  /// The armed tool. Selection edits; Hand only ever draws.
  final ToolMode tool;

  /// Whether each selected layer's anchor point is marked — the pin it turns
  /// on. Drawn while the Rotation tool is armed, where "about what?" is
  /// the question the picture has to answer.
  final bool showAnchors;

  /// Where the picture is drawn, and how big the comp is — the two halves of
  /// putting a guide, which is kept in comp pixels, on screen. Only
  /// snapping reads them.
  final Rect picture;
  final Size compSize;

  final VoidCallback onChanged;

  /// The engine's id for the view this is drawn over. With several views up,
  /// only the active one answers the arrow keys and a double-click.
  final int? viewId;

  /// Take the picture to a magnification, null for fit: the right-click
  /// menu's two zoom rows. The panel holds the magnification, so it applies it.
  final ValueChanged<double?>? onZoom;

  const ViewerGizmoLayer({
    super.key,
    required this.comp,
    required this.uiState,
    required this.boxes,
    required this.showControls,
    required this.tool,
    required this.onChanged,
    required this.picture,
    required this.compSize,
    this.showAnchors = false,
    this.viewId,
    this.onZoom,
    this.cameras = const [],
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

  /// Which key of [_acting]'s motion path is in hand, while one is.
  int? _motionKey;

  /// The layer under the pointer, drawn faintly so a click is predictable.
  UuidValue? _hover;

  /// The mask points that are selected, by [maskPointKey].
  ///
  /// Points, not layers: with a mask on the picture the same marquee that
  /// gathers layers gathers *vertices*, and a drag then moves them. Which of
  /// the two a gesture means is decided by what is under it — a press on a
  /// point edits the path, a press anywhere else is the layer's.
  final Set<String> _points = {};

  /// The handle in hand, while [_GizmoDrag.handle] is the drag, what it does
  /// to the point's other handle, and whether it is a fresh pair being pulled
  /// out of the point rather than a handle already there.
  PathHandle? _handleInHand;
  HandleLink _link = HandleLink.broken;
  bool _pullingOut = false;

  /// The mask point whose soft edge the Mask feather tool is dragging.
  ({LayerBox box, BridgeMask mask, int index})? _featherAt;

  /// The claim this layer displaced to take Delete for path points.
  bool Function()? _priorDeleteClaim;

  final PreviewThrottle _throttle = PreviewThrottle();

  /// Counts a second click in one place, for opening what a layer holds.
  final DoubleTap _taps = DoubleTap();

  /// The 3D handles a drag is on, as they stood when it began, and which of
  /// them is in hand.
  Handles3d? _set;
  GizmoPick? _part;

  /// Where the 3D drag last put its values, kept while the pointer is
  /// somewhere the axis cannot reach, such as past its vanishing point.
  Vec3? _last3d;

  /// The other selected layers a 3D drag moves with it, as they stood when
  /// it began, and whether the one with the handles rides along on one of
  /// them rather than being written itself.
  List<LayerBox> _carried = const [];
  bool _handlesRide = false;

  /// The layers a body drag leaves to a parent that is moving too.
  Set<UuidValue> _riders = const {};

  /// The 3D handle under the pointer, which set and which part.
  (BridgeTransformProp, GizmoPick)? _hot3d;

  /// Whether a body drag moves a 3D layer across the screen (Alt) rather
  /// than across its parent's flat.
  bool _onScreen = false;

  @override
  void initState() {
    super.initState();
    HardwareKeyboard.instance.addHandler(_onKey);
    // Chained, as the panels chain theirs: Delete with path points selected
    // is about the points, and anything else goes on down the chain.
    _priorDeleteClaim = widget.uiState.deleteClaim;
    widget.uiState.deleteClaim = _deleteClaim;
  }

  /// The rows a drag on the picture writes, for the others in a shared
  /// project: by the handle, the transform rows of the layers it acts on.
  /// A path point is its whole layer, since a path's row is not known
  /// here. A band changes nothing.
  List<String> _rowsOf(_GizmoDrag drag) {
    final props = switch (drag) {
      _GizmoDrag.none || _GizmoDrag.marquee => null,
      _GizmoDrag.move ||
      _GizmoDrag.motionKey =>
        const [BridgeTransformProp.positionX],
      _GizmoDrag.scale => const [BridgeTransformProp.scaleX],
      _GizmoDrag.rotate => const [BridgeTransformProp.rotation],
      _GizmoDrag.anchor => const [
          BridgeTransformProp.anchorX,
          BridgeTransformProp.positionX
        ],
      _GizmoDrag.points ||
      _GizmoDrag.handle ||
      _GizmoDrag.feather =>
        const <BridgeTransformProp>[],
      _GizmoDrag.axis3d => [
          _set?.props.first ?? BridgeTransformProp.positionX,
        ],
    };
    if (props == null) return const [];
    final acting = _acting ?? _set?.box;
    return [
      for (final box in acting == null ? _selected : [acting])
        if (props.isEmpty)
          '${box.id}'
        else
          for (final prop in props)
            '${transformPath('${box.id}')}/${prop.name}',
    ];
  }

  // A drag begins and ends with a `setState` here, so this hears each one.
  // Its rows are in this person's hand while it is under way. One that
  // someone else is changing is not dragged.
  @override
  void setState(VoidCallback fn) {
    super.setState(fn);
    final rows = _rowsOf(_drag);
    if (rows.isNotEmpty &&
        !rows.every(valuesInHand.value.contains) &&
        heldByOthers(rows)) {
      return _onPanCancel();
    }
    sayInHand(this, rows);
  }

  @override
  void dispose() {
    sayInHand(this, const []);
    HardwareKeyboard.instance.removeHandler(_onKey);
    if (widget.uiState.deleteClaim == _deleteClaim) {
      widget.uiState.deleteClaim = _priorDeleteClaim;
    }
    _throttle.cancel();
    super.dispose();
  }

  bool get _selectionTool => widget.tool.group == ToolGroup.select;

  /// The Pen's siblings, which edit a finished path: adding, removing and
  /// converting its points.
  bool get _vertexTool =>
      widget.tool == ToolMode.penAddVertex ||
      widget.tool == ToolMode.penDeleteVertex ||
      widget.tool == ToolMode.penConvertVertex ||
      widget.tool == ToolMode.penMaskFeather;

  /// Ctrl+Alt (Cmd+Option on a Mac) with the Selection tool is Convert
  /// vertex for the click, as in After Effects.
  bool get _convertChord {
    final keys = HardwareKeyboard.instance;
    return _selectionTool &&
        (keys.isControlPressed || keys.isMetaPressed) &&
        keys.isAltPressed;
  }

  /// Whether this is the view the keys and the double-click belong to. Every
  /// view on screen hears a key, and only one of them may answer it.
  bool get _activeView =>
      widget.viewId == null ||
      widget.uiState.views.active?.engineId == widget.viewId;

  /// The arrow keys nudge the selected layers by one composition pixel, ten
  /// with Shift, one undo step a press. Only with the Selection tool in hand
  /// and the Viewer the active panel, and never while something is being typed.
  /// A layer with a keyed position stays put, as it does under a drag.
  bool _onKey(KeyEvent event) {
    if (event is KeyUpEvent || !mounted || !_selectionTool) return false;
    final step = switch (event.logicalKey) {
      LogicalKeyboardKey.arrowLeft => const Offset(-1, 0),
      LogicalKeyboardKey.arrowRight => const Offset(1, 0),
      LogicalKeyboardKey.arrowUp => const Offset(0, -1),
      LogicalKeyboardKey.arrowDown => const Offset(0, 1),
      _ => null,
    };
    if (step == null) return false;
    final keys = HardwareKeyboard.instance;
    if (keys.isControlPressed || keys.isMetaPressed || keys.isAltPressed) {
      return false;
    }
    final ui = widget.uiState;
    if (lumitModalOpen ||
        lumitPopupOpen ||
        _drag != _GizmoDrag.none ||
        !_activeView ||
        ui.activePanel != Panel.viewer) {
      return false;
    }
    final focus = FocusManager.instance.primaryFocus;
    final focused = focus?.context;
    if (focus is ControlFocusNode ||
        (focused != null &&
            (focused.widget is EditableText ||
                focused.findAncestorWidgetOfExactType<EditableText>() !=
                    null))) {
      return false;
    }
    // A chord somebody has bound for themselves keeps its meaning.
    if (ui.keymap.actionFor(BridgeKeyContext.viewer, event) != null) {
      return false;
    }
    final by = step * (keys.isShiftPressed ? 10.0 : 1.0);
    final moves = [
      for (final box in _selected)
        if (box.draggable)
          if (nudgeStep(box, by) case final step?)
            (
              box,
              box.map.px + step.$1,
              box.map.py + step.$2,
              step.$3 == 0 ? null : box.map.depth!.z + step.$3,
            ),
    ];
    if (moves.isEmpty) return false;
    _place(moves);
    return true;
  }

  /// The boxes of the selected layers, in stacking order.
  List<LayerBox> get _selected {
    final ids = widget.uiState.selectedLayerIds;
    return [
      for (final box in widget.boxes)
        if (ids.contains(box.id)) box
    ];
  }

  /// The boxes to **outline**: the selected layers, plus any layer a picked
  /// property row belongs to.
  ///
  /// Picking a property is saying which layer is being worked on, so the
  /// picture should say so too — before this, keying a mask's opacity left the
  /// Viewer showing nothing at all, and there was no way to see which layer
  /// the curve on screen belonged to. Drawing only: what can be *dragged*
  /// stays the layer selection proper, so an outline never turns into a handle
  /// nobody asked for.
  /// The set itself lives on [LumitUiState.outlinedLayerIds], because the
  /// stage now decides from the same set whether to ask the engine where these
  /// layers' animated masks are — two readers of one rule rather than
  /// two copies of it.
  List<LayerBox> get _outlined {
    final ids = widget.uiState.outlinedLayerIds;
    return [
      for (final box in widget.boxes)
        if (ids.contains(box.id.toString())) box
    ];
  }

  /// The boxes whose points may be aimed at: the selected layers, plus the
  /// layer owning a mask or shape whose Path row is picked. Picking that row is
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

  /// The mask whose **Path** row is picked, or the shape item with any of its
  /// rows picked, if one is — the shape the author is editing, so it is the
  /// one whose points are offered even when the layer itself was never clicked.
  (UuidValue, UuidValue)? get _pathBeingEdited {
    for (final path in widget.uiState.selectedProperties.value) {
      final parts = path.split('/');
      // <layer>/masks/<mask>/path or <layer>/contents/<item>[/<row>]
      if ((parts.length == 4 &&
              parts[1] == 'masks' &&
              parts[3] == MaskValue.path.name) ||
          (parts.length >= 3 && parts[1] == 'contents')) {
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
    final move = _drag == _GizmoDrag.move ? _move : null;
    final sets =
        _drag == _GizmoDrag.move ? const <Handles3d>[] : _setsOf(selected);
    final held = _set;
    final part = _part;

    final painter = CustomPaint(
      key: const ValueKey('viewer-gizmo'),
      painter: _GizmoPainter(
        selected: widget.showControls ? outlined : const [],
        // The motion paths of the outlined layers, under the same switch as
        // the boxes (docs/07 §2.2 item 5; their own switch is owed).
        motionPaths: widget.showControls ? outlined : const [],
        keyInHand: _drag == _GizmoDrag.motionKey &&
                _acting != null &&
                _motionKey != null
            ? (layer: _acting!.id, index: _motionKey!, nudge: _delta)
            : null,
        hover: widget.showControls && _hover != null && _selectionTool
            ? _hoverBox()
            : null,
        handlesFor: widget.showControls && _selectionTool ? soleHandles : null,
        // The masks of what is selected: a mask you cannot see is a mask you
        // cannot judge, and until mask editing exists this outline is the only
        // sight of one on the picture.
        maskedBoxes: widget.showControls ? outlined : const [],
        selectedPoints: _points,
        pointNudge: _drag == _GizmoDrag.points ? _delta : Offset.zero,
        pointFrom: _origin,
        pathEdit: _drag == _GizmoDrag.handle ? _handleEdit() : null,
        maskInHand: _drag == _GizmoDrag.feather ? _featherNow() : null,
        onlyPath: _pathBeingEdited,
        anchors: widget.showControls && widget.showAnchors
            ? [
                for (final box in selected)
                  if (box.map.inFront(box.map.ax, box.map.ay))
                    box.anchorScreen +
                      (box.map.projected
                          ? Offset.zero
                          : move?.delta ?? Offset.zero),
              ]
            : const [],
        marquee: _drag == _GizmoDrag.marquee ? _marqueeRect() : null,
        moved: move?.delta ?? Offset.zero,
        // What the move has caught, across the picture. The accent, as a
        // guide is: §3.2 lets a guide be the one saturated mark over the
        // picture, and this is the same kind of line.
        snapX: move?.x,
        snapY: move?.y,
        picture: widget.picture,
        snapLine: t.accent,
        // `animated`, not `accent` (§3.1): the closed list gives that
        // colour to "this is selected or in hand", and **selected gizmo
        // handles** are named in it. The approved drawing agrees — the box
        // round the picture's selected layer and the mark on its anchor are
        // both drawn in the amber, not the clay — and §3.2 bans the accent
        // inside the Viewer's neutrality zone in the first place.
        accent: t.animated,
        hairline: t.hairlineStrong,
        surface: t.surface0,
        handles3d: sets,
        hot3d: _drag == _GizmoDrag.none ? _hot3d : null,
        held3d: held != null && part != null ? (held.props.first, part) : null,
        neutral: t.textPrimary,
        readout: switch (_drag) {
          _GizmoDrag.axis3d => _readout3d(),
          _GizmoDrag.move when _onScreen => _readoutAcross(),
          _ => const [],
        },
        pointer: _pointer,
        readoutStyle: t.small.copyWith(color: t.textPrimary),
        plate: t.surface0.withValues(alpha: 0.85),
      ),
    );

    // The Hand tool never edits: its boxes are a read-out, and the drag under
    // them belongs to the panel, which pans the picture with it.
    if (!_selectionTool && !_vertexTool) {
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
            onSecondaryTapUp: _onSecondaryTapUp,
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

  /// [box] as the gesture in flight would have it.
  ///
  /// The picture underneath is previewed at the value being dragged towards
  /// while the document still holds the old one, so a box built from the
  /// document lags the picture and only catches up on release. Three gestures
  /// can be in flight, never at once: this gizmo's own rotation knob and its
  /// scale handles, both local, and the Rotation tool's turn, which arrives on
  /// [LumitUiState.liveRotations] from another layer of the Viewer's stack.
  LayerBox _live(LayerBox box) {
    // A move seen through a camera is drawn where it puts the layer, not slid
    // across the screen with the pointer.
    if (_drag == _GizmoDrag.move &&
        box.map.projected &&
        box.draggable &&
        !_riders.contains(box.id) &&
        widget.uiState.selectedLayerIds.contains(box.id)) {
      final (x, y, z) = _movedPosition(box);
      return box.pivotedAt(Offset(box.map.ax, box.map.ay), Offset(x, y), z: z);
    }
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
            final (:position, :z) = panBehindFor(box, anchor);
            return box.pivotedAt(anchor, position, z: z);
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

  /// Whether the marquee in flight adds to the selection: Shift or Ctrl held
  /// as it started, the rule every other selection box keeps
  /// ([MarqueeSelect.onSelect]).
  bool _additive = false;

  // --- Snapping (docs/07 §2.2 item 6) ---------------------------------------
  //
  // The drag the *layer* takes, as against the travel the pointer asked for:
  // with the magnet on, the selection's box reaches for the guides — and for
  // the grid, while Snap to grid is ticked — in the Timeline magnet's own
  // grammar, measured in screen pixels so the magnification is the precision
  // control (`viewer_snap.dart`).
  //
  // **What caught it is drawn for as long as it holds.** A guide is a line on
  // the screen the whole time, so a layer arriving on one shows itself. A line
  // of the grid with the grid switched off, the frame's own edge and its
  // middle are not: nothing is there until something lands on it, which is
  // the Timeline's reason for marking its target, and it holds here too.

  /// The whole selection's box on screen, or null when nothing draggable is
  /// selected. Corners rather than sides, because a turned layer's box is a
  /// turned quad and what lines up with a guide is its extent.
  Rect? _selectionRect() => _rectOf(_selected.where((box) => box.draggable));

  /// The upright box round [boxes] on screen, or null when there are none.
  Rect? _rectOf(Iterable<LayerBox> boxes) {
    var left = double.infinity;
    var top = double.infinity;
    var right = double.negativeInfinity;
    var bottom = double.negativeInfinity;
    var any = false;
    for (final box in boxes) {
      for (final corner in box.corners) {
        if (!corner.dx.isFinite || !corner.dy.isFinite) continue;
        any = true;
        left = math.min(left, corner.dx);
        top = math.min(top, corner.dy);
        right = math.max(right, corner.dx);
        bottom = math.max(bottom, corner.dy);
      }
    }
    return any ? Rect.fromLTRB(left, top, right, bottom) : null;
  }

  /// The travel the move actually uses, and the lines that took it: the
  /// pointer's own, kept to the axis it has gone furthest along while Shift is
  /// held, then pulled onto the nearest line within the magnet's reach.
  ({Offset delta, double? x, double? y}) get _move {
    // Shift is the same lock it is on the anchor: one screen axis. As it
    // stood when the pointer last moved, which is the move that was drawn:
    // read as the button comes up, letting go of Shift a moment before the
    // mouse dropped the layer off the axis it had been shown on.
    final locked = _axisLocked;
    final delta = _drag == _GizmoDrag.move && locked
        ? constrainToAxis(_delta)
        : _delta;
    final free = (delta: delta, x: null, y: null);
    if (_drag != _GizmoDrag.move || delta == Offset.zero) return free;
    final tools = widget.uiState.tools;
    if (!tools.snapping ||
        snapSuspended(
            controlPressed: HardwareKeyboard.instance.isControlPressed)) {
      return free;
    }
    final rect = _selectionRect();
    if (rect == null) return free;
    List<double> targets(bool vertical) => viewerSnapTargets(
          guides: widget.uiState.guides,
          vertical: vertical,
          picture: widget.picture,
          compSize: widget.compSize,
          grid: tools.snapToGrid,
          divisions: ViewerOverlayPainter.divisions,
        );
    return snapViewerDrag(
      box: rect,
      delta: delta,
      // The axis Shift is holding reaches for nothing, or the magnet would
      // pull the layer off the line it is being kept to.
      verticals: locked && delta.dx == 0 ? const [] : targets(true),
      horizontals: locked && delta.dy == 0 ? const [] : targets(false),
    );
  }

  Offset get _moveDelta => _move.delta;

  void _setHover(UuidValue? id) {
    if (_hover == id) return;
    setState(() => _hover = id);
  }

  /// Track what a click would select. Only the layers that are *not* already
  /// selected highlight: a box already drawn in the accent needs no second
  /// mark, and highlighting it would read as a second state.
  void _onHover(PointerHoverEvent event) {
    if (_drag != _GizmoDrag.none) return;
    final hot = _pick3d(event.localPosition);
    final named = hot == null ? null : (hot.$1.props.first, hot.$2);
    if (named != _hot3d) setState(() => _hot3d = named);
    if (named != null) return _setHover(null);
    final hit = layerAtPoint(widget.boxes, event.localPosition);
    final ids = widget.uiState.selectedLayerIds;
    _setHover(hit == null || ids.contains(hit.id) ? null : hit.id);
  }

  /// A click selects what is under it — with Shift, adds to or removes from the
  /// selection; on empty space, selects nothing.
  void _onTapUp(TapUpDetails details) {
    // A click on a 3D handle is a drag that went nowhere, not a pick.
    if (_pick3d(details.localPosition) != null) return;
    final shift = HardwareKeyboard.instance.isShiftPressed;
    if (widget.showControls &&
        (_vertexTool || _convertChord) &&
        _vertexToolTap(details.localPosition)) {
      return;
    }
    // A click on a handle that went nowhere leaves everything as it was.
    if (widget.showControls &&
        pathHandleAt(_editablePointBoxes, details.localPosition, _points,
                only: _pathBeingEdited) !=
            null) {
      return;
    }
    // A mask point first: it sits on top of the layer it belongs to, and a
    // click on it means the point rather than the layer.
    final point = widget.showControls
        ? pathPointAt(_editablePointBoxes, details.localPosition,
            only: _pathBeingEdited)
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
      // A second click in the same place opens what the layer holds.
      if (_taps.tap(at: details.localPosition, slop: kDoubleTapSlop)) {
        _open(hit, details.localPosition);
      }
    }
    _setHover(null);
  }

  /// A double-click opens what the layer holds: a text layer's words, for
  /// typing, as a click with the Type tool does, and a precomp layer's
  /// composition, as its bar in the Timeline does. No other kind has anything
  /// to open.
  void _open(LayerBox hit, Offset at) {
    final ui = widget.uiState;
    BridgeLayerKind? kind;
    for (final entry in ui.model.heldLayers) {
      if (entry.layer.internallayerId == hit.id) kind = entry.info.kind;
    }
    if (kind == BridgeLayerKind.text) {
      // The Type layer of this view takes the request as the tool arms. If
      // nothing took it by the end of the frame it is dropped, so it cannot
      // open an edit the next time the tool is picked up.
      viewerTypeEditRequest = (view: widget.viewId, layer: hit.layer, at: at);
      WidgetsBinding.instance
          .addPostFrameCallback((_) => viewerTypeEditRequest = null);
      ui.tools.select(ToolMode.typeHorizontal);
    } else if (kind == BridgeLayerKind.precomp) {
      try {
        if (hit.layer.getSourceItem()
            case ItemReference_Composition(:final field0)) {
          ui.openNestedComp(hit.layer, field0);
        }
      } catch (_) {
        // A layer that has gone: nothing to open.
      }
    }
  }

  // --- The right-click menu ---------------------------------------------------

  /// The picture's own menu: every layer under the pointer by name, which is
  /// how one hidden under another is reached; Align, while something is
  /// selected; the two magnifications asked for most; and the view menu's
  /// switches, the same rows it draws.
  void _onSecondaryTapUp(TapUpDetails details) {
    final ui = widget.uiState;
    // The press moved a 3D viewport's view, so it was no click.
    if (ui.viewerNavigated) return;
    final under = [
      for (final box in widget.boxes)
        if (box.contains(details.localPosition)) box
    ];
    final names = {
      for (final entry in ui.model.heldLayers)
        entry.layer.internallayerId: entry.info.name,
    };
    final picked = ui.selectedLayerIds;
    final zoom = widget.onZoom;
    Widget flyout(List<Widget> rows) => FloatSurface(
          child: IntrinsicWidth(
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: rows,
            ),
          ),
        );
    // The empty tick slot, so every name in the menu starts on one line.
    Widget named(String text) => Row(children: [menuTick(false), Text(text)]);
    showMenuAt<void>(
      context: context,
      position: details.globalPosition,
      rows: (close) => [
        if (under.isNotEmpty)
          SubmenuRow(
            key: const ValueKey('viewer-menu-select'),
            closeParent: () => close(null),
            submenu: (dismiss) => flyout([
              for (final box in under)
                MenuRow(
                  key: ValueKey<String>('viewer-select-${box.id}'),
                  onPressed: () {
                    dismiss();
                    ui.setSelection([box.layer]);
                  },
                  child: Row(children: [
                    menuTick(picked.contains(box.id)),
                    Text(names[box.id] ?? ''),
                  ]),
                ),
            ]),
            child: named(l10n.viewerMenuSelect),
          ),
        if (picked.isNotEmpty)
          SubmenuRow(
            key: const ValueKey('viewer-menu-align'),
            closeParent: () => close(null),
            submenu: (dismiss) => flyout([
              for (final to in _Align.values)
                MenuRow(
                  key: ValueKey<String>('viewer-align-${to.name}'),
                  onPressed: () {
                    dismiss();
                    _align(to.x, to.y);
                  },
                  child: Text(to.title),
                ),
            ]),
            child: named(l10n.viewerMenuAlign),
          ),
        if (zoom != null) ...[
          MenuRow(
            key: const ValueKey('viewer-menu-fit'),
            onPressed: () {
              close(null);
              zoom(null);
            },
            child: named(l10n.menuFit),
          ),
          MenuRow(
            key: const ValueKey('viewer-menu-actual'),
            onPressed: () {
              close(null);
              zoom(1);
            },
            // A number, spelled the way the magnification picker spells it.
            child: named('100%'),
          ),
        ],
        for (final row in [
          ...viewerOverlayToggles(ui),
          (
            key: 'viewer-menu-checkerboard',
            text: l10n.tipTransparencyGrid,
            on: ui.viewerGrid,
            pick: () => ui.setViewerGrid(!ui.viewerGrid),
          ),
        ])
          viewerToggleRow(row, () => close(null)),
      ],
    );
  }

  /// Move each selected layer so a point of its box meets the same point of
  /// the composition, [x] and [y] from 0 to 1 across and down, null for an
  /// axis left alone. The box is the one the outline draws, so what lines up
  /// is what is seen. A layer with a keyed position stays where it is, as it
  /// does under a drag.
  void _align(double? x, double? y) {
    final moves = <(LayerBox, double, double, double?)>[];
    for (final box in _selected) {
      if (!box.draggable || _rectOf([box]) == null) continue;
      var (px, py) = (box.map.px, box.map.py);
      // Seen through a camera a move also changes how big the box looks, so
      // a few steps close in on the edge rather than landing near it.
      for (var i = 0; i < (box.map.projected ? 4 : 1); i++) {
        final moved = i == 0
            ? box
            : box.pivotedAt(Offset(box.map.ax, box.map.ay), Offset(px, py));
        final rect = _rectOf([moved]);
        if (rect == null) break;
        // Both measured from the composition's corner, at the view's scale.
        final scale = box.map.viewScale;
        final r = rect.shift(-box.map.origin);
        final frame = widget.compSize * scale;
        final d = Offset(
              x == null ? 0 : (frame.width - r.width) * x - r.left,
              y == null ? 0 : (frame.height - r.height) * y - r.top,
            ) /
            scale;
        final step = moved.map.positionStep(d);
        px += step.dx;
        py += step.dy;
      }
      moves.add((box, px, py, null));
    }
    _place(moves);
  }

  /// Write new Positions, in composition pixels: one op a layer for both axes,
  /// and the lot as one undo step. A move, a nudge and an align all land here.
  /// A z where one is given goes in the same op, and a keyed value takes a
  /// key at the playhead, as the 3D handles do.
  void _place(List<(LayerBox, double, double, double?)> moves) {
    if (moves.isEmpty) return;
    var landed = false;
    asOneUndoStep(
      // One layer is one op already, and needs no group round it.
      moves.length > 1
          ? Provider.of<LumitState>(context, listen: false).project
          : null,
      () {
        for (final (box, x, y, z) in moves) {
          // One op for both axes. x and y are separate properties in the
          // model, and writing them separately made one drag cost two undo
          // steps: Ctrl+Z put the layer back half way, along one axis, which
          // reads as the undo being broken rather than as two honest edits.
          try {
            BridgeScalar keyed(BridgeScalar? was, double v) =>
                scalarWithValueAt(was ?? BridgeScalar.static_(v), v,
                    widget.comp, widget.uiState.playheadFrame.value);
            final tf = box.transform;
            final deep =
                z != null && tf?.positionZ is! BridgeScalar_Expression;
            box.layer.setTransforms(
              props: [
                BridgeTransformProp.positionX,
                BridgeTransformProp.positionY,
                if (deep) BridgeTransformProp.positionZ,
              ],
              values: [
                keyed(tf?.positionX, x),
                keyed(tf?.positionY, y),
                if (deep) keyed(tf?.positionZ, z),
              ],
            );
            landed = true;
          } catch (_) {
            // A layer deleted meanwhile. The rest still move.
          }
        }
      },
    );
    if (landed) widget.onChanged();
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
    // Except against a path point aimed at squarely. A shape layer's
    // box *is* its art's bounding box, so its outermost points sit exactly on
    // the corners the scale handles occupy — and a handle's reach is twice a
    // point's, so every corner of a drawn square was a scale and never an edit.
    // A press inside a point's own, tighter reach means the point.
    final selected = _selected;
    final only = _pathBeingEdited;
    final aimedAt = widget.showControls
        ? pathPointAt(_editablePointBoxes, at, only: only)
        : null;

    // Convert vertex drags a handle on its own, or pulls a pair out of a
    // point. The other two tools only click, and none of the three moves a
    // layer or sweeps a marquee.
    if (widget.showControls && widget.tool == ToolMode.penMaskFeather) {
      _takeFeather(at);
      return;
    }
    final converting =
        widget.tool == ToolMode.penConvertVertex || _convertChord;
    if (_vertexTool || (converting && aimedAt != null)) {
      if (!widget.showControls || !converting) return;
      final handle =
          pathHandleAt(_editablePointBoxes, at, _points, only: only);
      if (handle != null) {
        _takeHandle(handle, HandleLink.broken);
      } else if (aimedAt != null) {
        _points
          ..clear()
          ..add(aimedAt.key);
        _takeHandle(
          (
            box: aimedAt.box,
            key: aimedAt.key,
            index: aimedAt.index,
            side: HandleSide.leaving,
          ),
          HandleLink.mirrored,
          pullingOut: true,
        );
      }
      return;
    }

    // A handle of a selected point, unless the press is squarely on a point:
    // a short handle ends close to its own point, and the point must stay
    // reachable. A smooth point's other handle turns with the one in hand.
    if (widget.showControls &&
        (aimedAt == null ||
            pathPointAt(_editablePointBoxes, at, slop: 4, only: only) ==
                null)) {
      final handle =
          pathHandleAt(_editablePointBoxes, at, _points, only: only);
      if (handle != null) {
        final v = _vertexOf(handle);
        _takeHandle(
            handle,
            v != null && isSmoothVertex(v)
                ? HandleLink.turned
                : HandleLink.broken);
        return;
      }
    }

    // A key on a selected layer's motion path: a press within its own tight
    // reach drags that key, whatever layer's body sits under it.
    if (aimedAt == null && widget.showControls) {
      final key = motionKeyAt(selected, at);
      if (key != null) {
        setState(() {
          _acting = key.box;
          _motionKey = key.index;
          _drag = _GizmoDrag.motionKey;
        });
        return;
      }
    }

    // A 3D arrow or square beats the box's own handles, all but the anchor,
    // which sits in the middle of them and is aimed at more tightly.
    if (aimedAt == null &&
        !(selected.length == 1 &&
            selected.single.scalable &&
            selected.single.handleHit(at) == GizmoHandle.anchor)) {
      if (_pick3d(at) case (final set, final part)) {
        setState(() {
          _set = set;
          _part = part;
          _last3d = set.at;
          // A point of interest moves alone. Position takes the rest of the
          // selection with it, as a body drag does.
          final carried = set.props.first == BridgeTransformProp.poiX
              ? const <LayerBox>[]
              : [
                  for (final box in [...selected, ...widget.cameras])
                    if (box.id != set.box.id && box.transform != null) box
                ];
          final riders = ridingAlong(
              {set.box.id, for (final box in carried) box.id},
              widget.uiState.model.heldLayers);
          _carried = [
            for (final box in carried)
              if (!riders.contains(box.id)) box
          ];
          _handlesRide = riders.contains(set.box.id);
          _drag = _GizmoDrag.axis3d;
          _hot3d = null;
          _hover = null;
        });
        return;
      }
    }

    if (aimedAt == null && selected.length == 1 && selected.single.scalable) {
      final handle = selected.single.handleHit(at);
      if (handle == GizmoHandle.anchor &&
          panBehindHitsKeys(selected.single)) {
        Provider.of<LumitState>(context, listen: false)
            .postNotice(l10n.anchorKeyedRefused);
        return;
      }
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
      // points, which it could not do if the press had already dropped
      // the layer they belong to.
      final keys = HardwareKeyboard.instance;
      _additive =
          keys.isShiftPressed || keys.isControlPressed || keys.isMetaPressed;
      setState(() => _drag = _GizmoDrag.marquee);
      return;
    }

    // Dragging a layer that is not selected selects it first — otherwise the
    // gesture would move something the user had not chosen. A layer already in
    // the selection leaves the selection alone, so a set can be dragged as one.
    if (!widget.uiState.selectedLayerIds.contains(hit.id)) {
      widget.uiState.setSelection([hit.layer]);
    }
    // A keyed position has no one value for the drag to add to: the press
    // has picked the layer, and its keys are dragged on the path instead.
    if (!hit.draggable) {
      _setHover(null);
      return;
    }
    setState(() {
      _drag = _GizmoDrag.move;
      _onScreen = HardwareKeyboard.instance.isAltPressed;
      _riders = ridingAlong(widget.uiState.selectedLayerIds,
          widget.uiState.model.heldLayers);
      _hover = null;
    });
  }

  /// Whether Shift was held at the pointer's last move ([_move]).
  bool _axisLocked = false;

  void _onPanUpdate(DragUpdateDetails details) {
    setState(() {
      _pointer = details.localPosition;
      _delta += details.delta;
      _axisLocked = HardwareKeyboard.instance.isShiftPressed;
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
      case _GizmoDrag.handle:
        final edit = _handleEdit();
        if (edit != null) _previewEdit((_) => edit);
      case _GizmoDrag.feather:
        _previewFeather();
      case _GizmoDrag.motionKey:
        _previewMotionKey();
      case _GizmoDrag.axis3d:
        _preview3d();
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
      case _GizmoDrag.handle:
        final edit = _handleEdit();
        if (edit != null && _delta != Offset.zero) _commitEdit((_) => edit);
      case _GizmoDrag.feather:
        _commitFeather();
      case _GizmoDrag.motionKey:
        _commitMotionKey();
      case _GizmoDrag.marquee:
        _commitMarquee();
      case _GizmoDrag.axis3d:
        _commit3d();
      case _GizmoDrag.none:
        break;
    }
    _onPanCancel();
  }

  void _onPanCancel() {
    _throttle.cancel();
    if (_drag == _GizmoDrag.axis3d) {
      widget.uiState.liveTransforms.value = const {};
    }
    setState(() {
      _drag = _GizmoDrag.none;
      _delta = Offset.zero;
      _acting = null;
      _handle = null;
      _handleInHand = null;
      _pullingOut = false;
      _featherAt = null;
      _motionKey = null;
      _set = null;
      _part = null;
      _last3d = null;
      _carried = const [];
      _handlesRide = false;
      _riders = const {};
      _onScreen = false;
    });
  }

  // --- Move -----------------------------------------------------------------

  /// A live preview, but only for a single layer: the engine patches one
  /// layer's transform into a clone of the document per request (the preview
  /// path), so a set being dragged shows the picture move on release
  /// instead. The boxes follow the pointer either way, which is what makes the
  /// gesture readable.
  void _previewMove() {
    final selected = _selected;
    if (selected.length != 1) return;
    _throttle.request(() => _sendMovePreview(selected.single));
  }

  void _sendMovePreview(LayerBox box) {
    final (x, y, z) = _movedPosition(box);
    _sendPreview(box,
        (tf) => transformWith(tf, positionX: x, positionY: y, positionZ: z));
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

  /// Where a dragged layer's Position lands: the magnet's travel, not the
  /// pointer's. The preview, the box on screen and the committed op all
  /// come through here, so the picture cannot land anywhere but where the
  /// wireframe said it would. With Alt a 3D layer goes across the screen at
  /// its depth instead, so z moves too.
  (double, double, double?) _movedPosition(LayerBox box) {
    final m = box.map;
    if (_onScreen && m.projected) {
      final at = (m.px, m.py, m.depth!.z);
      final d = Gizmo3d.of(m, at: at, local: const [], mode: GizmoAxes.view)
          ?.onScreen(_origin, _origin + _moveDelta);
      if (d != null) return (at.$1 + d.$1, at.$2 + d.$2, at.$3 + d.$3);
    }
    final step = m.positionStep(_moveDelta / m.viewScale);
    return (m.px + step.dx, m.py + step.dy, null);
  }

  void _commitMove() {
    if (_delta == Offset.zero) return;
    final moves = <(LayerBox, double, double, double?)>[];
    for (final box in _selected) {
      if (!box.draggable || _riders.contains(box.id)) continue;
      final (x, y, z) = _movedPosition(box);
      moves.add((box, x, y, z));
    }
    _place(moves);
  }

  // --- 3D arrows and squares ------------------------------------------------

  /// The 3D handles on screen, on the selected 3D layer, light or camera
  /// picked first, in the axes the toolbar is set to.
  List<Handles3d> _setsOf(List<LayerBox> selected) {
    if (!widget.showControls || !_selectionTool) return const [];
    final primary = primary3d([...selected, ...widget.cameras],
        widget.uiState.selectedLayer.value?.internallayerId);
    return primary == null
        ? const []
        : handles3dOf(primary, widget.uiState.tools.axes);
  }

  /// The 3D handle under [at], and its set.
  (Handles3d, GizmoPick)? _pick3d(Offset at) {
    for (final set in _setsOf(_selected)) {
      final part = set.gizmo.pick(at, turning: false);
      if (part != null) return (set, part);
    }
    return null;
  }

  /// Where the 3D drag in hand puts its three values. Shift keeps it to
  /// whole steps of ten.
  Vec3? _moved3d() {
    final set = _set, part = _part;
    if (set == null || part == null) return null;
    final step = HardwareKeyboard.instance.isShiftPressed ? 10.0 : null;
    final d = switch (part.kind) {
      GizmoKind.arrow =>
        set.gizmo.along(part.axis, _origin, _pointer, step: step),
      GizmoKind.screen => set.gizmo.onScreen(_origin, _pointer),
      _ => set.gizmo.across(part.axis, _origin, _pointer, step: step),
    };
    if (d == null) return _last3d;
    final at = set.at;
    return _last3d = (at.$1 + d.$1, at.$2 + d.$2, at.$3 + d.$3);
  }

  /// Every layer the 3D drag moves and where to: the one with the handles,
  /// then the rest of the selection by the same move in the comp, each in
  /// its own parent's space. A 2D layer takes the move across the picture.
  List<(LayerBox, List<BridgeTransformProp>, Vec3, Vec3)> _moves3d() {
    final set = _set, to = _moved3d();
    if (set == null || to == null) return const [];
    final d = (to.$1 - set.at.$1, to.$2 - set.at.$2, to.$3 - set.at.$3);
    final inComp = parentToComp(set.box.map, d);
    const position = [
      BridgeTransformProp.positionX,
      BridgeTransformProp.positionY,
      BridgeTransformProp.positionZ,
    ];
    return [
      if (!_handlesRide) (set.box, set.props, set.at, to),
      for (final box in _carried)
        if (_carriedTo(box, inComp) case (final from, final to))
          (box, position, from, to),
    ];
  }

  (Vec3, Vec3)? _carriedTo(LayerBox box, Vec3 inComp) {
    final m = box.map;
    final tf = box.transform;
    if (tf == null ||
        [tf.positionX, tf.positionY, if (m.projected) tf.positionZ]
            .any((s) => s is BridgeScalar_Expression)) {
      return null;
    }
    if (!m.projected) {
      if (!box.draggable) return null;
      final step = m.positionStep(Offset(inComp.$1, inComp.$2));
      return ((m.px, m.py, 0), (m.px + step.dx, m.py + step.dy, 0));
    }
    final d = compToParent(m, inComp);
    if (d == null) return null;
    final from = (m.px, m.py, m.depth!.z);
    return (from, (from.$1 + d.$1, from.$2 + d.$2, from.$3 + d.$3));
  }

  /// [tf] with a 3D drag's three values in it.
  BridgeTransform _with3d(
          BridgeTransform tf, List<BridgeTransformProp> props, Vec3 to) =>
      props.first == BridgeTransformProp.poiX
          ? transformWith(tf, pointOfInterest: to)
          : transformWith(tf,
              positionX: to.$1, positionY: to.$2, positionZ: to.$3);

  /// The boxes and the picture follow the drag: the boxes are rebuilt from
  /// the provisional transforms, children included, and one layer moving
  /// alone is previewed as every other drag previews.
  void _preview3d() {
    final moves = _moves3d();
    final live = {
      for (final (box, props, _, to) in moves)
        box.id: _with3d(box.transform!, props, to),
    };
    if (live.isEmpty) return;
    widget.uiState.liveTransforms.value = live;
    if (moves.length != 1) return;
    final layer = moves.single.$1.layer;
    final patched = live.values.single;
    _throttle.request(() {
      try {
        widget.comp.renderFrameWithTransformPreview(
          frame: BigInt.from(widget.uiState.playheadFrame.value),
          scale: widget.uiState.viewerScale,
          layer: layer,
          transform: patched,
        );
      } catch (_) {
        // A preview is a courtesy; the move still lands.
      }
    });
  }

  /// One op a layer for the values the drag changed, the lot as one undo
  /// step. A keyed value takes a key at the playhead, a still one stays
  /// still.
  void _commit3d() {
    if (_delta == Offset.zero) return;
    final moves = _moves3d();
    final frame = widget.uiState.playheadFrame.value;
    var landed = false;
    asOneUndoStep(
      moves.length > 1
          ? Provider.of<LumitState>(context, listen: false).project
          : null,
      () {
        for (final (box, props, from, to) in moves) {
          final was = [from.$1, from.$2, from.$3];
          final now = [to.$1, to.$2, to.$3];
          final write = <BridgeTransformProp>[];
          final values = <BridgeScalar>[];
          for (var i = 0; i < 3; i++) {
            final scalar = scalarOf(box.transform!, props[i]);
            if (scalar == null || (now[i] - was[i]).abs() < 1e-9) continue;
            write.add(props[i]);
            values.add(scalarWithValueAt(scalar, now[i], widget.comp, frame));
          }
          if (write.isEmpty) continue;
          try {
            box.layer.setTransforms(props: write, values: values);
            landed = true;
          } catch (_) {
            // A layer deleted mid-drag. The rest still move.
          }
        }
      },
    );
    if (landed) widget.onChanged();
  }

  /// What the 3D drag has changed so far, for the readout by the pointer.
  List<String> _readout3d() {
    final set = _set, to = _last3d;
    if (set == null || to == null) return const [];
    return _readout(
        set.props.first == BridgeTransformProp.poiX
            ? l10n.transformPointOfInterest
            : l10n.transformPosition,
        set.at,
        to);
  }

  /// The same for an Alt drag across the view, on the layer the handles
  /// would sit on.
  List<String> _readoutAcross() {
    final box = primary3d(
        _selected, widget.uiState.selectedLayer.value?.internallayerId);
    if (box == null || !box.draggable) return const [];
    final (x, y, z) = _movedPosition(box);
    final m = box.map;
    return _readout(l10n.transformPosition, (m.px, m.py, m.depth!.z),
        (x, y, z ?? m.depth!.z));
  }

  List<String> _readout(String what, Vec3 from, Vec3 to) {
    final d = [to.$1 - from.$1, to.$2 - from.$2, to.$3 - from.$3];
    return [
      what,
      for (var i = 0; i < 3; i++)
        if (d[i].abs() >= 0.05) '${'xyz'[i]}  ${gizmoAmount(d[i])}',
    ];
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
    // One op for both axes, for the same reason a move is.
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
    final map = box.map;
    // Seen through a camera the turn is measured on the layer's own plane,
    // round its anchor, so the knob stays under the pointer on a tilted
    // layer.
    Offset onPlane(Offset at) {
      final p = map.layerOf(at);
      return Offset((p.dx - map.ax) * map.sx, (p.dy - map.ay) * map.sy);
    }

    return rotationForDrag(
      anchor: map.projected ? Offset.zero : box.anchorScreen,
      from: map.projected ? onPlane(_origin) : _origin,
      to: map.projected ? onPlane(_pointer) : _pointer,
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
  /// modifiers the Anchor point tool has: Shift locks the drag to one
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
    _throttle.request(() => _sendPreview(
          box,
          (tf) => transformPannedBehind(box, tf, anchor),
        ));
  }

  void _commitAnchor() {
    final box = _acting;
    final anchor = _anchorNow();
    if (box == null || anchor == null || _delta == Offset.zero) return;
    try {
      // One op for all of it: half of this edit moves the picture, which is
      // the one thing panning behind promises not to do.
      setPannedBehind(box, anchor,
          comp: widget.comp, frame: widget.uiState.playheadFrame.value);
      widget.onChanged();
    } catch (_) {
      // The layer went away mid-drag.
    }
  }

  // --- Mask points ----------------------------------------------------------

  /// Write every dragged point's new position.
  ///
  /// The drag is a screen delta; each point is moved in its **own layer's**
  /// space, so a selection spanning two layers with different transforms still
  /// moves together on screen. One `set_mask` per mask, which is one undo step
  /// per mask — the same rule the razor follows for a multi-layer cut.
  void _commitPoints() {
    if (_delta == Offset.zero || _points.isEmpty) return;
    _commitEdit((box) =>
        _pointsMoved(_points, pointDeltaIn(box, _delta, from: _origin)));
  }

  /// Write an edit to every path it touches: a point drag, a handle, or a
  /// point added, removed or converted. [editFor] gives the edit in each
  /// layer's own terms.
  void _commitEdit(VertexEdit Function(LayerBox box) editFor) {
    var landed = false;
    // The same boxes a point can be grabbed from, so a picked Path row's
    // layer gets the write even when the layer isn't selected.
    for (final box in _editablePointBoxes) {
      final edit = editFor(box);
      for (final mask in box.masks) {
        final moved = maskEdited(box, mask, edit);
        if (moved == null) continue;
        try {
          // The playhead goes with it: on a mask whose shape is keyed, the
          // drag belongs to the key sitting there rather than to the static
          // path, which `path_at` would ignore.
          box.layer.setMask(
            mask: moved,
            at: widget.comp
                .timeOfFrame(frame: widget.uiState.playheadFrame.value),
          );
          landed = true;
          // **A keyed shape edit shows itself in the Timeline**. The
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
      // by id; shape contents are a whole list (`SetShapeContents`), so a
      // layer's items are rebuilt together and committed once. That is still
      // one undo step per layer. The engine moves the layer with the art's box
      // in that same op, so the points nobody dragged stay where they are.
      final contents = shapeContentsEdited(box, edit);
      if (contents != null) {
        try {
          // The playhead goes with it, for the mask's reason above: on an item
          // whose shape is keyed, the drag belongs to the key sitting there
          // rather than to the still path.
          box.layer.setShapeContents(
            contents: contents,
            at: widget.comp
                .timeOfFrame(frame: widget.uiState.playheadFrame.value),
          );
          landed = true;
          // A keyed shape edit shows itself in the Timeline: the row
          // the new key belongs to is the one the author now wants to see.
          for (final item in contents) {
            if (item.pathKeys.isEmpty) continue;
            widget.uiState.requestSelectProperty(
              '${box.id}/contents/${item.id}/${ShapeValue.path.name}',
            );
            break;
          }
        } catch (_) {
          // The layer or its art went away mid-drag; the rest still move.
        }
      }
    }
    if (landed) widget.onChanged();
  }

  /// The picture under a point drag, while it is being dragged.
  ///
  /// A point drag used to show only its wireframe and leave the picture until
  /// the release — the same "no room in the preview call" the transform drags
  /// once had, and the reason a path edit was a guess until you let go. The
  /// engine patches **one** layer's paths into a clone of the document per
  /// request, so this previews a single layer, exactly as a move does.
  void _previewPoints() => _previewEdit((box) =>
      _pointsMoved(_points, pointDeltaIn(box, _delta, from: _origin)));

  /// The picture under any path edit in flight, as [_previewPoints] has it.
  void _previewEdit(VertexEdit Function(LayerBox box) editFor) {
    final touched = [
      for (final box in _editablePointBoxes)
        if (shapeContentsEdited(box, editFor(box)) != null ||
            box.masks.any((m) => maskEdited(box, m, editFor(box)) != null))
          box,
    ];
    if (touched.length != 1) return;
    final box = touched.single;
    _throttle.request(() => _sendEditPreview(box, editFor(box)));
  }

  void _sendEditPreview(LayerBox box, VertexEdit edit) {
    final contents = shapeContentsEdited(box, edit);
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
      // ponytail: masks and art are previewed one or the other, art first —
      // one request patches one of them. The ceiling is precise: with both
      // kinds of point in the drag, the art moves live and the mask outline
      // stays where it was until the pointer is released, at which point it
      // jumps to catch up. The trigger is a marquee that caught a mask point
      // and a shape-art point on the same layer, then a drag — the only way to
      // reach it, and the reason it is rare. The fix is one request carrying
      // both patches rather than a choice between them.
      final masks = [
        for (final mask in box.masks)
          maskEdited(box, mask, edit) ?? mask,
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

  // --- Path handles and the Pen's siblings ---------------------------------

  void _takeHandle(PathHandle handle, HandleLink link,
      {bool pullingOut = false}) {
    setState(() {
      _handleInHand = handle;
      _link = link;
      _pullingOut = pullingOut;
      _drag = _GizmoDrag.handle;
    });
  }

  /// The point a handle belongs to, as the document has it.
  BridgeVertex? _vertexOf(PathHandle h) {
    for (final path in pathsOf(h.box)) {
      if (h.index < path.vertices.length && path.keyOf(h.index) == h.key) {
        return path.vertices[h.index];
      }
    }
    return null;
  }

  /// What the handle drag in flight does to the path it is on. Alt breaks a
  /// smooth point's handles apart, as in After Effects.
  VertexEdit? _handleEdit() {
    final h = _handleInHand;
    if (h == null) return null;
    final d = pointDeltaIn(h.box, _delta, from: _origin);
    final link = _pullingOut
        ? HandleLink.mirrored
        : HardwareKeyboard.instance.isAltPressed
            ? HandleLink.broken
            : _link;
    return (keyOf, vertices, closed) {
      if (h.index >= vertices.length || keyOf(h.index) != h.key) return null;
      final from =
          _pullingOut ? Offset.zero : handleOf(vertices[h.index], h.side);
      return verticesWithHandle(vertices, h.index, h.side, from + d, link);
    };
  }

  /// Delete with path points selected, in the Viewer: the points go, and a
  /// path left with too few to be a path goes with them, as in After Effects.
  bool _deleteClaim() {
    final ui = widget.uiState;
    if (!mounted ||
        _points.isEmpty ||
        ui.activePanel != Panel.viewer ||
        !_activeView) {
      return _priorDeleteClaim?.call() ?? false;
    }
    final at = widget.comp.timeOfFrame(frame: ui.playheadFrame.value);
    var landed = false;
    List<BridgeVertex> kept(
            List<BridgeVertex> vertices, String Function(int) keyOf) =>
        [
          for (var i = 0; i < vertices.length; i++)
            if (!_points.contains(keyOf(i))) vertices[i],
        ];
    for (final box in _editablePointBoxes) {
      for (final mask in box.masks) {
        final left =
            kept(mask.vertices, (i) => maskPointKey(box.id, mask.id, i));
        if (left.length == mask.vertices.length) continue;
        try {
          if (left.length < (mask.closed ? 3 : 2)) {
            box.layer.deleteMask(id: mask.id);
          } else {
            box.layer.setMask(
                mask: maskEdited(box, mask, (_, __, ___) => left)!, at: at);
          }
          landed = true;
        } catch (_) {
          // The mask went away; the rest still go.
        }
      }
      var touched = false;
      final contents = <BridgeShapeItem>[];
      for (final item in box.shapeContents) {
        final left =
            kept(item.vertices, (i) => shapePointKey(box.id, item.id, i));
        if (left.length == item.vertices.length) {
          contents.add(item);
          continue;
        }
        touched = true;
        if (left.length >= (item.closed ? 3 : 2)) {
          contents.add(shapeItemWith(item, vertices: left));
        }
      }
      if (touched) {
        try {
          box.layer.setShapeContents(contents: contents, at: at);
          landed = true;
        } catch (_) {
          // The layer went away.
        }
      }
    }
    setState(_points.clear);
    if (!landed) return _priorDeleteClaim?.call() ?? false;
    widget.onChanged();
    return true;
  }

  /// The Mask feather tool's press: a mask point, or either edge of the soft
  /// band drawn round it, starts a drag that sets that point's width.
  void _takeFeather(Offset at) {
    ({LayerBox box, BridgeMask mask, int index})? best;
    var nearest = gizmoAnchorSlop / 2;
    for (final box in _editablePointBoxes) {
      for (final mask in box.masks) {
        if (!showsPath(box, mask.id, _pathBeingEdited)) continue;
        for (var i = 0; i < mask.vertices.length; i++) {
          final width = stillFeatherAt(mask, i);
          if (width == null) continue;
          final v = mask.vertices[i];
          final n = featherNormalAt(mask, i) * (width / 2);
          for (final p in [Offset(v.x, v.y), Offset(v.x, v.y) + n, Offset(v.x, v.y) - n]) {
            final d = (box.map.toScreen(p.dx, p.dy) - at).distance;
            if (d <= nearest) {
              nearest = d;
              best = (box: box, mask: mask, index: i);
            }
          }
        }
      }
    }
    if (best == null) return;
    setState(() {
      _featherAt = best;
      _points
        ..clear()
        ..add(maskPointKey(best!.box.id, best.mask.id, best.index));
      _drag = _GizmoDrag.feather;
    });
  }

  /// The mask in hand as the feather drag has it: the soft band's edge
  /// follows the pointer, so the width is twice the pointer's distance from
  /// the path, measured square to it.
  BridgeMask? _featherNow() {
    final f = _featherAt;
    if (f == null) return null;
    final v = f.mask.vertices[f.index];
    final p = f.box.map.layerOf(_pointer);
    final reach = (p - Offset(v.x, v.y));
    final n = featherNormalAt(f.mask, f.index);
    final width = 2 * (reach.dx * n.dx + reach.dy * n.dy).abs();
    return maskWithFeatherAt(f.mask, f.index, width);
  }

  void _previewFeather() {
    final f = _featherAt;
    final mask = _featherNow();
    if (f == null || mask == null) return;
    _throttle.request(() {
      try {
        widget.comp.renderFrameWithMaskPreview(
          frame: BigInt.from(widget.uiState.playheadFrame.value),
          scale: widget.uiState.viewerScale,
          layer: f.box.layer,
          masks: [
            for (final m in f.box.masks) m.id == mask.id ? mask : m,
          ],
        );
      } catch (_) {
        // A preview is a courtesy, the release still lands.
      }
    });
  }

  void _commitFeather() {
    final f = _featherAt;
    final mask = _featherNow();
    if (f == null || mask == null || _delta == Offset.zero) return;
    try {
      f.box.layer.setMask(
        mask: mask,
        at: widget.comp.timeOfFrame(frame: widget.uiState.playheadFrame.value),
      );
      widget.onChanged();
    } catch (_) {
      // The mask went away mid-drag.
    }
  }

  /// A click with Add, Delete or Convert vertex. True when it changed a path.
  bool _vertexToolTap(Offset at) {
    // Mask feather only drags; its click selects, as the Selection tool's does.
    if (widget.tool == ToolMode.penMaskFeather) return false;
    final boxes = _editablePointBoxes;
    final only = _pathBeingEdited;
    final point = pathPointAt(boxes, at, only: only);
    if (widget.tool == ToolMode.penAddVertex) {
      // A click on a point is left to select it.
      if (point != null) return false;
      final edge = pathEdgeAt(boxes, at, only: only);
      if (edge == null) return false;
      final first = edge.path.keyOf(edge.segment);
      _commitEdit((_) => (keyOf, vertices, closed) =>
          keyOf(edge.segment) == first
              ? verticesWithPointAdded(vertices, edge.segment, edge.t)
              : null);
      setState(() => _points
        ..clear()
        ..add(edge.path.keyOf(edge.segment + 1)));
      return true;
    }
    if (point == null) return false;
    bool mine(String Function(int) keyOf) => keyOf(point.index) == point.key;
    if (widget.tool == ToolMode.penDeleteVertex) {
      _commitEdit((_) => (keyOf, vertices, closed) => mine(keyOf)
          ? verticesWithoutPoint(vertices, point.index, closed)
          : null);
      // The points after it have moved up one, so their names have changed.
      setState(_points.clear);
    } else {
      _commitEdit((_) => (keyOf, vertices, closed) => mine(keyOf)
          ? verticesConverted(vertices, point.index, closed)
          : null);
      setState(() => _points
        ..clear()
        ..add(point.key));
    }
    return true;
  }

  // --- Motion-path keys (docs/07 §2.4) ---------------------------------------

  /// The key in hand and where it is being dragged to, in comp pixels: its
  /// own point plus the pointer's travel at the view's scale.
  (BridgeMotionKey, Offset)? _motionKeyNow() {
    final box = _acting;
    final index = _motionKey;
    if (box == null || index == null) return null;
    final keys = box.motionPath?.keys;
    if (keys == null || index >= keys.length) return null;
    final key = keys[index];
    return (
      key,
      Offset(key.x, key.y) +
          box.map.positionStep(_delta / box.map.viewScale,
              from: motionKeyScreen(box, key),
              z: box.map.projected ? key.z : null),
    );
  }

  /// The picture under a key drag: the frame on screen, rendered with the
  /// curve as it would be with the key moved. Where the playhead is outside
  /// the span the key changes, the engine hands back the frame it has.
  void _previewMotionKey() {
    final box = _acting;
    final now = _motionKeyNow();
    if (box == null || now == null) return;
    _throttle.request(() =>
        _sendPreview(box, (tf) => transformWithMotionKey(tf, now.$1, now.$2)));
  }

  /// Write the moved key: one op for whichever axes have a key at that time,
  /// so one drag is one undo step, exactly as a move of a still layer is.
  ///
  /// ponytail: the path itself catches up on release, when the document's
  /// change refreshes it; only the key's box follows the pointer. A live
  /// re-sample per tick is one more sync read on the same throttle if it is
  /// ever wanted.
  void _commitMotionKey() {
    final box = _acting;
    final now = _motionKeyNow();
    if (box == null || now == null || _delta == Offset.zero) return;
    final (key, to) = now;
    try {
      final tf = transformWithMotionKey(box.layer.getTransform(), key, to);
      box.layer.setTransforms(
        props: [
          if (key.xIndex != null) BridgeTransformProp.positionX,
          if (key.yIndex != null) BridgeTransformProp.positionY,
        ],
        values: [
          if (key.xIndex != null) tf.positionX,
          if (key.yIndex != null) tf.positionY,
        ],
      );
      widget.onChanged();
    } catch (_) {
      // The layer went away mid-drag.
    }
  }

  // --- Marquee --------------------------------------------------------------

  void _commitMarquee() {
    final rect = _marqueeRect();
    // A stray click that happened to be read as a tiny drag should not clear a
    // selection the user just made another way.
    if (rect.width < 3 && rect.height < 3) return;

    // A sweep over a selected layer's mask points gathers **points**:
    // with a path on screen that is what a rubber band means, and the layers
    // are already selected anyway. With none caught it is the layer sweep it
    // has always been.
    if (widget.showControls) {
      final caughtPoints =
          pathPointsInRect(_editablePointBoxes, rect, only: _pathBeingEdited);
      if (caughtPoints.isNotEmpty) {
        setState(() {
          if (!_additive) _points.clear();
          _points.addAll(caughtPoints);
        });
        return;
      }
    }
    if (!_additive) setState(_points.clear);
    final caught = layersInsideRect(widget.boxes, rect);
    final layers = <LayerReference>[
      if (_additive)
        for (final box in _selected) box.layer,
      for (final box in caught)
        if (!_additive || !widget.uiState.selectedLayerIds.contains(box.id))
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
  double? positionZ,
  double? scaleX,
  double? scaleY,
  double? rotation,
  double? rotationX,
  double? rotationY,
  Vec3? pointOfInterest,
  Vec3? orientation,
}) {
  BridgeScalar put(double? value, BridgeScalar keep) =>
      value == null ? keep : BridgeScalar.static_(value);
  final camera = tf.camera;
  final depth = tf.depth;
  return BridgeTransform(
    anchorX: put(anchorX, tf.anchorX),
    anchorY: put(anchorY, tf.anchorY),
    positionX: put(positionX, tf.positionX),
    positionY: put(positionY, tf.positionY),
    positionZ: put(positionZ, tf.positionZ),
    scaleX: put(scaleX, tf.scaleX),
    scaleY: put(scaleY, tf.scaleY),
    rotation: put(rotation, tf.rotation),
    rotationX: put(rotationX, tf.rotationX),
    rotationY: put(rotationY, tf.rotationY),
    opacity: tf.opacity,
    camera: camera == null
        ? null
        : BridgeCameraChannels(
            poiX: put(pointOfInterest?.$1, camera.poiX),
            poiY: put(pointOfInterest?.$2, camera.poiY),
            poiZ: put(pointOfInterest?.$3, camera.poiZ),
            zoom: camera.zoom,
            focusDistance: camera.focusDistance,
            aperture: camera.aperture,
            blurLevel: camera.blurLevel,
          ),
    depth: depth == null
        ? null
        : BridgeDepthChannels(
            orientationX: put(orientation?.$1, depth.orientationX),
            orientationY: put(orientation?.$2, depth.orientationY),
            orientationZ: put(orientation?.$3, depth.orientationZ),
            anchorZ: depth.anchorZ,
            scaleZ: depth.scaleZ,
            afterEffectsOrder: depth.afterEffectsOrder,
          ),
  );
}

/// Everything the gizmo draws: the selected boxes, the hovered one, the
/// handles, and the marquee.
class _GizmoPainter extends CustomPainter {
  final List<LayerBox> selected;

  /// The boxes whose motion path is drawn — those that have one, that is.
  final List<LayerBox> motionPaths;

  /// The motion-path key being dragged: whose, which, and how far it has
  /// travelled so far — so its box follows the pointer before the document
  /// hears about it.
  final ({UuidValue layer, int index, Offset nudge})? keyInHand;

  /// The boxes whose masks are outlined.
  final List<LayerBox> maskedBoxes;

  /// The mask points that are selected, and how far a drag has moved them so
  /// far — so the path follows the pointer before the document hears about it.
  final Set<String> selectedPoints;
  final Offset pointNudge;

  /// Where the point drag was pressed, which a layer seen through a camera
  /// measures the drag from.
  final Offset pointFrom;

  /// A handle drag in flight, drawn on the path it is changing.
  final VertexEdit? pathEdit;

  /// A mask whose soft edge is being dragged, drawn as the drag has it.
  final BridgeMask? maskInHand;

  /// The layer and path whose Path row is picked, which is then the only path
  /// of its layer drawn.
  final (UuidValue, UuidValue)? onlyPath;
  final LayerBox? hover;
  final LayerBox? handlesFor;

  /// Where to mark an anchor point, in screen space.
  final List<Offset> anchors;
  final Rect? marquee;
  final Offset moved;

  /// The lines a move in flight has caught, on screen, one an axis; null
  /// where it has caught nothing. Drawn across [picture] in [snapLine].
  final double? snapX;
  final double? snapY;
  final Rect picture;
  final Color snapLine;

  /// The colour a selected box, its handles and its anchor mark are drawn in:
  /// `animated` (§3.1), which is the name the field keeps for its history.
  final Color accent;
  final Color hairline;
  final Color surface;

  /// The 3D handles, the one under the pointer and the one in hand, each
  /// named by its set's first value, and the centre ring's colour.
  final List<Handles3d> handles3d;
  final (BridgeTransformProp, GizmoPick)? hot3d;
  final (BridgeTransformProp, GizmoPick)? held3d;
  final Color neutral;

  /// What a 3D drag has changed, drawn by [pointer].
  final List<String> readout;
  final Offset pointer;
  final TextStyle readoutStyle;
  final Color plate;

  const _GizmoPainter({
    required this.selected,
    required this.motionPaths,
    required this.keyInHand,
    required this.maskedBoxes,
    required this.selectedPoints,
    required this.pointNudge,
    required this.pointFrom,
    this.pathEdit,
    this.maskInHand,
    this.onlyPath,
    required this.hover,
    required this.handlesFor,
    required this.anchors,
    required this.marquee,
    required this.moved,
    required this.snapX,
    required this.snapY,
    required this.picture,
    required this.snapLine,
    required this.accent,
    required this.hairline,
    required this.surface,
    required this.handles3d,
    required this.hot3d,
    required this.held3d,
    required this.neutral,
    required this.readout,
    required this.pointer,
    required this.readoutStyle,
    required this.plate,
  });

  @override
  void paint(Canvas canvas, Size size) {
    // What the move has caught, under the boxes so the edge that landed on it
    // stays legible, the way the Timeline draws its own.
    final caught = Paint()
      ..color = snapLine
      ..strokeWidth = 1;
    if (snapX case final x?) {
      canvas.drawLine(
          Offset(x, picture.top), Offset(x, picture.bottom), caught);
    }
    if (snapY case final y?) {
      canvas.drawLine(
          Offset(picture.left, y), Offset(picture.right, y), caught);
    }

    // The layer a click would take: the same box, drawn faintly. Under the
    // selection, so a selected box is never dimmed by a hover on top of it.
    final hovered = hover;
    if (hovered != null) {
      _outline(canvas, hovered.outline, accent.withValues(alpha: 0.35));
    }

    for (final box in selected) {
      final shift = _shift(box);
      _outline(canvas, [for (final c in box.outline) c + shift], accent);
    }

    final handles = handlesFor;
    if (handles != null && handles.wholeInFront) {
      final moved = _shift(handles);
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
      // The pivot, which is now a handle in its own right: drawn as
      // the anchor's ring-and-cross rather than a square, so it never reads as
      // a ninth scale handle.
      paintAnchorMark(
          canvas, handles.handleAt(GizmoHandle.anchor) + moved, accent);
    }

    List<BridgeVertex> live(String Function(int) keyOf,
            List<BridgeVertex> vertices, bool closed) =>
        pathEdit?.call(keyOf, vertices, closed) ?? vertices;
    for (final box in maskedBoxes) {
      for (final mask in box.masks) {
        if (!showsPath(box, mask.id, onlyPath)) continue;
        String keyOf(int i) => maskPointKey(box.id, mask.id, i);
        _pathOutline(
          canvas,
          box,
          live(keyOf, mask.vertices, mask.closed),
          mask.closed,
          keyOf,
          feather: stillHalfFeather(
              maskInHand?.id == mask.id ? maskInHand! : mask),
        );
      }
      // A shape layer's own art gets the same outline and the same vertices, so
      // drawn art can be seen point by point and corrected rather than redrawn.
      for (final item in box.offeredItems) {
        if (!showsPath(box, item.id, onlyPath)) continue;
        String keyOf(int i) => shapePointKey(box.id, item.id, i);
        _pathOutline(
          canvas,
          box,
          live(keyOf, item.vertices, item.closed),
          item.closed,
          keyOf,
          // Art coordinates, a bounding box away from the layer's own.
          art: true,
        );
      }
    }

    for (final anchor in anchors) {
      paintAnchorMark(canvas, anchor, accent);
    }

    for (final box in motionPaths) {
      _motionPath(canvas, box);
    }

    final held = held3d;
    for (final set in handles3d) {
      final named = set.props.first;
      if (held != null && held.$1 != named) continue;
      paintGizmo3d(
        canvas,
        set.gizmo,
        turning: false,
        neutral: neutral,
        outline: surface,
        hot: hot3d?.$1 == named ? hot3d?.$2 : null,
        held: held?.$2,
      );
    }
    paintGizmoReadout(canvas, pointer, readout,
        style: readoutStyle, plate: plate);

    final band = marquee;
    if (band != null) paintMarquee(canvas, band, accent);
  }

  /// A layer's motion path (docs/07 §2.4): the sampled line, a dot per frame
  /// so their spacing reads as speed, a box per key — hollow when merely
  /// there, filled while in hand — and each key's tangent handles as a line to
  /// a hollow circle.
  void _motionPath(Canvas canvas, LayerBox box) {
    final path = box.motionPath;
    if (path == null) return;
    final line = Paint()
      ..color = accent
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;
    for (final run in motionPathRuns(box)) {
      if (run.length >= 2) {
        canvas.drawPath(Path()..addPolygon(run, false), line);
      }
    }
    final points = motionPathScreen(box);
    canvas.drawPoints(
      PointMode.points,
      points,
      Paint()
        ..color = accent.withValues(alpha: 0.6)
        ..strokeWidth = 3
        ..strokeCap = StrokeCap.round,
    );
    final inHand = keyInHand;
    for (var i = 0; i < path.keys.length; i++) {
      final key = path.keys[i];
      if (!_keyInFront(box, key)) continue;
      final held = inHand != null && inHand.layer == box.id && inHand.index == i;
      final nudge = held ? inHand.nudge : Offset.zero;
      final at = motionKeyScreen(box, key) + nudge;
      final z = box.map.projected ? key.z : null;
      for (final handle in [key.handleIn, key.handleOut]) {
        if (handle == null || !box.map.parentInFront(handle[0], handle[1], z)) {
          continue;
        }
        final h = compToScreen(box, handle[0], handle[1], z) + nudge;
        canvas.drawLine(at, h, line);
        canvas.drawCircle(h, 3, Paint()..color = surface);
        canvas.drawCircle(h, 3, line);
      }
      final rect = Rect.fromCenter(center: at, width: 6, height: 6);
      canvas.drawRect(rect, Paint()..color = held ? accent : surface);
      canvas.drawRect(rect, line);
    }
  }

  /// How far a move in flight has carried [box] on screen. A box seen through
  /// a camera is drawn where the move puts it instead, since perspective
  /// makes a move more than a slide across the screen.
  Offset _shift(LayerBox box) => box.map.projected ? Offset.zero : moved;

  void _outline(Canvas canvas, List<Offset> corners, Color colour) {
    if (corners.length < 2) return;
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
  /// Serves a mask and a shape layer's own art alike: the two
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
    final projected = box.map.projected;
    final moved = _shift(box);
    // A selected point follows the pointer while it is being dragged, so the
    // path bends live rather than jumping on release. Seen through a camera
    // the drag is carried on the layer, where the release writes it.
    final drag = projected
        ? pointDeltaIn(box, pointNudge, from: pointFrom)
        : Offset.zero;
    bool picked(int i) => selectedPoints.contains(keyOf(i));
    Offset nudgeFor(int i) =>
        picked(i) && !projected ? pointNudge : Offset.zero;
    Offset onLayer(int i, double dx, double dy) {
      final v = vertices[i];
      final n = picked(i) ? drag : Offset.zero;
      return Offset(v.x + dx + n.dx, v.y + dy + n.dy);
    }

    Offset screen(double x, double y) =>
        art ? box.shapePoint(x, y) : box.map.toScreen(x, y);
    Offset shown(Offset p) => screen(p.dx, p.dy) + moved;
    // How far in front of the eye a point of the path is, on the layer.
    double depth(Offset p) => art
        ? box.map.depthOf(p.dx - box.artOrigin.dx, p.dy - box.artOrigin.dy)
        : box.map.depthOf(p.dx, p.dy);
    bool seen(int i) => depth(onLayer(i, 0, 0)) > viewerNear;
    Offset at(int i) => shown(onLayer(i, 0, 0)) + nudgeFor(i);
    Offset out(int i) => shown(onLayer(
            i, vertices[i].tanOutX, vertices[i].tanOutY)) +
        nudgeFor(i);
    Offset into(int i) =>
        shown(onLayer(i, vertices[i].tanInX, vertices[i].tanInY)) +
        nudgeFor(i);

    canvas.drawPath(
      projected
          ? _walkedPath(vertices, closed, onLayer, shown, depth)
          : bezierPath(
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

    // **How wide the soft edge is, either side of the path**. A pair
    // of dimmer lines rather than a second solid one, because they are a guide
    // to a width and not a path anybody can grab: they say where the feather
    // reaches, which is the only way a feather that varies from point to point
    // can be seen before the frame is drawn.
    if (feather != null &&
        vertices.length >= 2 &&
        Iterable<int>.generate(vertices.length).every(seen)) {
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

    // A selected point's handles: a line out to each, ending in a dot that
    // can be dragged. A handle folded into its point has nothing to show.
    final stem = Paint()
      ..color = accent
      ..strokeWidth = 1;
    for (var i = 0; i < vertices.length; i++) {
      if (!seen(i) || !picked(i)) continue;
      final v = vertices[i];
      for (final (tan, end) in [
        (Offset(v.tanInX, v.tanInY), into(i)),
        (Offset(v.tanOutX, v.tanOutY), out(i)),
      ]) {
        if (tan.distance < 1e-9) continue;
        canvas.drawLine(at(i), end, stem);
        canvas.drawCircle(end, 3, Paint()..color = surface);
        canvas.drawCircle(
            end,
            3,
            Paint()
              ..color = accent
              ..strokeWidth = 1
              ..style = PaintingStyle.stroke);
      }
    }

    // Every vertex, so a path can be seen point by point: hollow when
    // it is merely there, filled when it is selected — the same "outline means
    // available, fill means chosen" the keyframe diamonds use.
    for (var i = 0; i < vertices.length; i++) {
      if (!seen(i)) continue;
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

  /// A path through a camera. Perspective bends a curve the way it bends the
  /// layer, so each span is walked on the layer and every step is put through
  /// the camera, rather than the handles alone. Steps behind the eye are cut
  /// out, the line ending a hair in front of it.
  Path _walkedPath(
    List<BridgeVertex> vertices,
    bool closed,
    Offset Function(int i, double dx, double dy) onLayer,
    Offset Function(Offset layerPoint) shown,
    double Function(Offset layerPoint) depth,
  ) {
    final path = Path();
    final spans = closed && vertices.length > 2
        ? vertices.length
        : vertices.length - 1;
    Offset? last;
    var lastDepth = 0.0;
    var drawing = false;
    var whole = true;
    void step(Offset p) {
      final d = depth(p) - viewerNear;
      if (d < 0) whole = false;
      final was = last;
      if (was != null && (d >= 0) != (lastDepth >= 0)) {
        final cut = shown(was + (p - was) * (lastDepth / (lastDepth - d)));
        if (drawing) {
          path.lineTo(cut.dx, cut.dy);
        } else {
          path.moveTo(cut.dx, cut.dy);
        }
        drawing = d >= 0;
      }
      if (d >= 0) {
        final at = shown(p);
        if (drawing) {
          path.lineTo(at.dx, at.dy);
        } else {
          path.moveTo(at.dx, at.dy);
          drawing = true;
        }
      }
      last = p;
      lastDepth = d;
    }

    step(onLayer(0, 0, 0));
    for (var i = 0; i < spans; i++) {
      final j = (i + 1) % vertices.length;
      final p0 = onLayer(i, 0, 0);
      final p1 = onLayer(i, vertices[i].tanOutX, vertices[i].tanOutY);
      final p2 = onLayer(j, vertices[j].tanInX, vertices[j].tanInY);
      final p3 = onLayer(j, 0, 0);
      // A step for every eight pixels the span reaches on screen, up to
      // sixteen: a small path needs few, and a big shape layer's many spans
      // stay cheap.
      final reach = (shown(p1) - shown(p0)).distance +
          (shown(p2) - shown(p1)).distance +
          (shown(p3) - shown(p2)).distance;
      final steps = reach.isFinite ? (reach / 8).ceil().clamp(1, 16) : 16;
      for (var k = 1; k <= steps; k++) {
        final t = k / steps;
        final u = 1 - t;
        step(p0 * (u * u * u) +
            p1 * (3 * u * u * t) +
            p2 * (3 * u * t * t) +
            p3 * (t * t * t));
      }
    }
    if (whole && closed && vertices.length > 2) path.close();
    return path;
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
