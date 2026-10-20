// The layer↔screen transform the Viewer gizmo shares — a pure, unit-tested port
// of the egui `LayerMap` (crates/lumit-ui/src/shell/timeline/mod.rs:30-85), the
// mapping the egui mask / anchor / shape / pen overlays all build through.
//
// In plain terms: a layer sits in the composition with a position, an anchor
// point, a scale and a rotation. To draw a handle on screen we have to push a
// point of the layer through that transform and then through the Viewer's own
// fit/zoom placement; to turn a mouse position back into a layer coordinate we
// run the whole thing in reverse. This module holds exactly those two maps and
// nothing else, so the gizmo widget stays free of trigonometry and the maths
// can be checked against hand-computed cases without a widget tree.
//
// Grounding (egui `LayerMap`):
//   to_screen(p):  d = (p - anchor) * scale
//                  r = rotate(d, rot)
//                  screen = origin + (position + r) * viewScale
//   layer_of(s):   c = (s - origin) / viewScale
//                  d = c - position
//                  r = rotate(d, -rot)       (inverse rotation)
//                  layer = r / scale + anchor
// `origin` is the fitted picture's top-left; `viewScale` converts comp pixels to
// screen pixels (the fitted rect's width / the comp's pixel width). `scale` is a
// factor (the transform stores a percentage, so the caller divides by 100), and
// egui floors it at 1e-6 so the inverse never divides by zero.

import 'dart:math' as math;
import 'dart:ui' show Offset;

import 'package:flutter/widgets.dart' show Matrix4;

/// A scale factor that can be flipped but never vanishes.
///
/// **Negative scale is a real value**: dragging a handle past the anchor turns
/// the layer over, which is how every editor mirrors a layer, and the map used
/// to floor the factor at a hair above zero — so a layer could be squashed to
/// nothing and never turned over. Only *zero* is barred, because the inverse
/// map divides by it; the sign is kept.
double nonZeroScale(double factor) {
  const smallest = 1e-6;
  if (factor.abs() >= smallest) return factor;
  return factor.isNegative ? -smallest : smallest;
}

/// Where a parent chain puts a layer's Position space in the comp, flattened to
/// the picture: `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
typedef ParentPlacement = ({
  double a,
  double b,
  double c,
  double d,
  double e,
  double f,
});

/// The evaluated 2D transform of a layer at the playhead, composed with the
/// Viewer's fitted-image placement. Construct it once per paint with
/// [ViewerLayerMap.of] and use [toScreen] / [layerOf] to move points across the
/// boundary.
class ViewerLayerMap {
  /// Position (comp pixels, from the comp's top-left — the same frame `origin`
  /// lives in).
  final double px, py;

  /// Anchor point (layer pixels).
  final double ax, ay;

  /// Scale factors (the transform's percentage / 100), floored at 1e-6.
  final double sx, sy;

  /// Rotation, pre-computed as its sine and cosine (radians).
  final double sin, cos;

  /// The fitted picture's top-left in screen (local) coordinates.
  final Offset origin;

  /// Comp pixels → screen pixels (the fitted rect width / the comp pixel width).
  final double viewScale;

  /// The parent chain's placement, null for a layer with no parent. Position,
  /// and everything under it, is in the parent's space, not the comp's.
  final ParentPlacement? parent;

  /// How the picture sees a 3D layer, null for a 2D one. With it every map
  /// here goes through the camera, so the box, the handles and the paths sit
  /// where the picture has them.
  final ViewerDepth? depth;

  ViewerLayerMap({
    required this.px,
    required this.py,
    required this.ax,
    required this.ay,
    required this.sx,
    required this.sy,
    required this.sin,
    required this.cos,
    required this.origin,
    required this.viewScale,
    this.parent,
    this.depth,
  });

  /// Build the map from the layer's evaluated transform values (as the snapshot
  /// reads them back): [scaleXPercent]/[scaleYPercent] are percentages (100 =
  /// full), [rotationDegrees] is in degrees, matching the engine's storage and
  /// egui's `LayerMap::of`.
  factory ViewerLayerMap.of({
    required double positionX,
    required double positionY,
    required double anchorX,
    required double anchorY,
    required double scaleXPercent,
    required double scaleYPercent,
    required double rotationDegrees,
    required Offset origin,
    required double viewScale,
    ParentPlacement? parent,
    ViewerDepth? depth,
  }) {
    final rot = rotationDegrees * math.pi / 180.0;
    return ViewerLayerMap(
      px: positionX,
      py: positionY,
      ax: anchorX,
      ay: anchorY,
      sx: nonZeroScale(scaleXPercent / 100.0),
      sy: nonZeroScale(scaleYPercent / 100.0),
      sin: math.sin(rot),
      cos: math.cos(rot),
      origin: origin,
      viewScale: viewScale,
      parent: parent,
      depth: depth,
    );
  }

  /// The same map with the layer turned to [rotationDegrees] instead.
  ///
  /// What a turn *in flight* looks like: the picture is previewed at the new
  /// angle, so the wireframe over it has to be too, and rebuilding the whole
  /// map from the document would only give back the angle the drag is leaving.
  ViewerLayerMap turnedTo(double rotationDegrees) {
    final rot = rotationDegrees * math.pi / 180.0;
    return ViewerLayerMap(
      px: px,
      py: py,
      ax: ax,
      ay: ay,
      sx: sx,
      sy: sy,
      sin: math.sin(rot),
      cos: math.cos(rot),
      origin: origin,
      viewScale: viewScale,
      parent: parent,
      depth: depth,
    );
  }

  /// The same map with the layer scaled to [scaleXPercent] / [scaleYPercent]
  /// instead — what a scale *in flight* looks like, for the same reason
  /// [turnedTo] exists.
  ViewerLayerMap scaledTo(double scaleXPercent, double scaleYPercent) =>
      ViewerLayerMap(
        px: px,
        py: py,
        ax: ax,
        ay: ay,
        sx: nonZeroScale(scaleXPercent / 100.0),
        sy: nonZeroScale(scaleYPercent / 100.0),
        sin: sin,
        cos: cos,
        origin: origin,
        viewScale: viewScale,
        parent: parent,
        depth: depth,
      );

  /// The same map with the layer's pivot moved to ([anchorX], [anchorY]) and
  /// its position set to [position] — the pair a pan-behind writes together.
  ///
  /// What a pivot drag in flight looks like: the anchor mark moves and
  /// the picture, box and all, deliberately does not.
  ViewerLayerMap pivotedAt(
    double anchorX,
    double anchorY, {
    required Offset position,
    double? z,
  }) =>
      ViewerLayerMap(
        px: position.dx,
        py: position.dy,
        ax: anchorX,
        ay: anchorY,
        sx: sx,
        sy: sy,
        sin: sin,
        cos: cos,
        origin: origin,
        viewScale: viewScale,
        parent: parent,
        depth: z == null ? depth : depth?.at(z),
      );

  /// Whether this layer is seen through a camera rather than flat.
  bool get projected => depth != null;

  /// Parent space `(x, y, z, 1)` to `(X, Y, W)` in comp pixels, row by row:
  /// the view's rows times the parent chain.
  late final List<double> _parentRows = depth!.parentRows();

  /// The same rows, for the 3D handles, which work in the parent's space.
  List<double> get parentRows => _parentRows;

  /// The layer's own plane `(x, y, 1)` to `(X, Y, W)`: the rows above times
  /// the layer's own placement, which has no z of its own because the layer
  /// is flat.
  late final List<double> _plane = () {
    final o = inParent(0, 0);
    final ex = inParent(1, 0);
    final ey = inParent(0, 1);
    final r = _parentRows;
    return [
      for (var row = 0; row < 3; row++) ...[
        _dot(r, row, ex.$1 - o.$1, ex.$2 - o.$2, ex.$3 - o.$3, 0),
        _dot(r, row, ey.$1 - o.$1, ey.$2 - o.$2, ey.$3 - o.$3, 0),
        _dot(r, row, o.$1, o.$2, o.$3, 1),
      ],
    ];
  }();

  late final List<double>? _planeInverse = _invert3(_plane);

  late final (double, double, double, double) _turns = () {
    final rx = (depth?.rotationX ?? 0) * math.pi / 180;
    final ry = (depth?.rotationY ?? 0) * math.pi / 180;
    return (math.sin(rx), math.cos(rx), math.sin(ry), math.cos(ry));
  }();

  /// Whether the layer is turned out of its parent's flat by its X or Y
  /// rotation, so a point moved on it moves in depth too.
  bool get tilted =>
      projected && (_turns.$1.abs() > 1e-9 || _turns.$3.abs() > 1e-9);

  /// Where the layer point ([x], [y]) is in its parent's space, depth and
  /// all: the compositor's T · Ry · Rx · Rz · S · T(-anchor), worked through
  /// by hand. A 2D layer's is at depth 0.
  (double, double, double) inParent(double x, double y) {
    final (sinX, cosX, sinY, cosY) = _turns;
    final dx = (x - ax) * sx;
    final dy = (y - ay) * sy;
    final zx = dx * cos - dy * sin;
    final zy = dx * sin + dy * cos;
    final (yy, yz) = (zy * cosX, zy * sinX);
    return (
      px + zx * cosY + yz * sinY,
      py + yy,
      (depth?.z ?? 0) - zx * sinY + yz * cosY,
    );
  }

  /// How far in front of the eye the parent-space point ([x], [y], [z]) is,
  /// as [depthOf] measures it. [z] is the layer's own Position z unless
  /// given.
  double parentDepthOf(double x, double y, [double? z]) => projected
      ? _dot(_parentRows, 2, x, y, z ?? depth!.z, 1)
      : 1;

  /// Whether the parent-space point ([x], [y], [z]) is in front of the eye.
  bool parentInFront(double x, double y, [double? z]) =>
      parentDepthOf(x, y, z) > viewerNear;

  /// Comp pixels, still to be divided by [w], to screen.
  Offset _shown(double x, double y, double w) {
    final safe = w.abs() < 1e-9 ? 1e-9 : w;
    return origin + Offset(x / safe, y / safe) * viewScale;
  }

  /// How far in front of the eye the layer point ([x], [y]) is, as the depth
  /// over the camera's zoom: at or below 0 behind it. A 2D layer is 1
  /// everywhere.
  double depthOf(double x, double y) {
    if (!projected) return 1;
    final h = _plane;
    return h[6] * x + h[7] * y + h[8];
  }

  /// Whether the layer point ([x], [y]) is in front of the eye.
  bool inFront(double x, double y) => depthOf(x, y) > viewerNear;

  /// How far away the layer is under the screen point [pos], as the depth
  /// over the camera's zoom, or null where the layer there is behind the eye
  /// or seen edge on. Nearer is smaller. A 2D layer is 1 everywhere.
  double? depthAt(Offset pos) {
    if (!projected) return 1;
    final inv = _planeInverse;
    if (inv == null) return null;
    final c = (pos - origin) / viewScale;
    final s = inv[6] * c.dx + inv[7] * c.dy + inv[8];
    return s > 1e-12 ? 1 / s : null;
  }

  /// Layer space → screen (local) coordinate.
  Offset toScreen(double x, double y) {
    if (projected) {
      final h = _plane;
      return _shown(h[0] * x + h[1] * y + h[2], h[3] * x + h[4] * y + h[5],
          h[6] * x + h[7] * y + h[8]);
    }
    final dx = (x - ax) * sx;
    final dy = (y - ay) * sy;
    final rx = dx * cos - dy * sin;
    final ry = dx * sin + dy * cos;
    return parentToScreen(px + rx, py + ry);
  }

  /// A point in the parent's space (where Position and a motion path live)
  /// → screen. Without a parent that space is the comp. [z] is a 3D layer's
  /// depth there, its own Position z unless given.
  Offset parentToScreen(double x, double y, [double? z]) {
    if (projected) {
      final r = _parentRows;
      final at = z ?? depth!.z;
      return _shown(_dot(r, 0, x, y, at, 1), _dot(r, 1, x, y, at, 1),
          _dot(r, 2, x, y, at, 1));
    }
    final p = parent;
    final c = p == null
        ? Offset(x, y)
        : Offset(p.a * x + p.c * y + p.e, p.b * x + p.d * y + p.f);
    return origin + c * viewScale;
  }

  /// Screen (local) → the parent's space, the inverse of [parentToScreen].
  /// A parent scaled to nought has no inverse and is read as no parent. A 3D
  /// layer's answer is on the flat of its parent's space at depth [z], its
  /// own Position z unless given, which is where a move keeps it.
  Offset parentOf(Offset pos, [double? z]) {
    final c = (pos - origin) / viewScale;
    if (projected) {
      final r = _parentRows;
      final at = z ?? depth!.z;
      final inv = _invert3([
        for (var row = 0; row < 3; row++) ...[
          r[row * 4],
          r[row * 4 + 1],
          r[row * 4 + 2] * at + r[row * 4 + 3],
        ],
      ]);
      return inv == null ? Offset(px, py) : _dehom(inv, c);
    }
    final p = parent;
    final det = p == null ? 0.0 : p.a * p.d - p.b * p.c;
    if (p == null || det == 0) return c;
    final x = c.dx - p.e;
    final y = c.dy - p.f;
    return Offset((p.d * x - p.c * y) / det, (p.a * y - p.b * x) / det);
  }

  /// A move of [compDelta] comp pixels on the picture, as the change it makes
  /// to Position. Seen through a camera a step is longer the further away it
  /// is taken, so a 3D layer's is measured from [from] on screen at depth
  /// [z], its own Position unless given.
  Offset positionStep(Offset compDelta, {Offset? from, double? z}) {
    if (!projected) {
      return parentOf(origin + compDelta * viewScale) - parentOf(origin);
    }
    final base = from ?? parentToScreen(px, py, z);
    return parentOf(base + compDelta * viewScale, z) - parentOf(base, z);
  }

  /// Screen (local) → layer space coordinate (the inverse of [toScreen]).
  /// Seen through a camera this is where the pointer's ray meets the layer,
  /// in front of the eye or not, which [depthAt] says.
  Offset layerOf(Offset pos) {
    if (projected) {
      final inv = _planeInverse;
      return inv == null
          ? Offset(ax, ay)
          : _dehom(inv, (pos - origin) / viewScale);
    }
    final c = parentOf(pos);
    final dx = c.dx - px;
    final dy = c.dy - py;
    // Inverse rotation (transpose of the forward matrix).
    final rx = dx * cos + dy * sin;
    final ry = -dx * sin + dy * cos;
    return Offset(rx / sx + ax, ry / sy + ay);
  }

  /// The scale percentages a corner/edge handle at layer-space offset
  /// ([dxFromAnchor], [dyFromAnchor]) from the anchor implies when dragged so
  /// that handle lands under screen point [pointer], with the anchor, rotation
  /// and position held fixed. Returns `(scaleXPercent, scaleYPercent)`.
  ///
  /// Derivation: for a handle at layer offset `(dx, dy)` from the anchor,
  /// `toScreen(handle) - toScreen(anchor) = rotate(diag(sx, sy)·(dx, dy)) ·
  /// viewScale`. Un-rotating the screen delta and dividing by `(dx, dy)` recovers
  /// each axis' factor. An axis with a zero offset (an edge handle, or the anchor
  /// sitting on that edge) keeps its current factor — that axis cannot be
  /// resolved and must not move.
  (double, double) scaleForHandle({
    required double dxFromAnchor,
    required double dyFromAnchor,
    required Offset pointer,
  }) {
    final double ux, uy;
    if (projected) {
      // On the layer's own plane, which is where the handle is dragged.
      final at = layerOf(pointer);
      ux = (at.dx - ax) * sx;
      uy = (at.dy - ay) * sy;
    } else {
      final at = parentOf(pointer);
      final ex = at.dx - px;
      final ey = at.dy - py;
      // Un-rotate the screen delta into the layer's un-scaled frame.
      ux = ex * cos + ey * sin;
      uy = -ex * sin + ey * cos;
    }
    final newSx = dxFromAnchor.abs() < 1e-9 ? sx : ux / dxFromAnchor;
    final newSy = dyFromAnchor.abs() < 1e-9 ? sy : uy / dyFromAnchor;
    return (newSx * 100.0, newSy * 100.0);
  }
}

/// How near the eye a point may be and still be drawn, as the depth over the
/// camera's zoom: a hair in front of it. Anything nearer is cut off.
const double viewerNear = 1e-3;

/// How the picture sees one 3D layer: the view it is looked at through, its
/// parents in full, and the parts of its own transform a flat map has no room
/// for.
class ViewerDepth {
  /// Comp space `(x, y, z, 1)` to `(X, Y, W)`, three rows of four, as the
  /// engine's `viewProjection` answers it. Asked for only once something is
  /// drawn or aimed at, so playback with nothing outlined asks nothing.
  final List<double> Function() view;

  /// The parent chain, parent space to comp space, null with no parent.
  final Matrix4? parent;

  /// Position z, and the x and y rotations in degrees.
  final double z;
  final double rotationX;
  final double rotationY;

  const ViewerDepth({
    required this.view,
    required this.z,
    required this.rotationX,
    required this.rotationY,
    this.parent,
  });

  /// The same layer at Position z [z].
  ViewerDepth at(double z) => ViewerDepth(
        view: view,
        z: z,
        rotationX: rotationX,
        rotationY: rotationY,
        parent: parent,
      );

  /// The view's rows times the parent chain.
  List<double> parentRows() {
    final rows = view();
    final m = parent?.storage;
    if (m == null) return rows;
    return [
      for (var row = 0; row < 3; row++)
        for (var col = 0; col < 4; col++)
          _dot(rows, row, m[col * 4], m[col * 4 + 1], m[col * 4 + 2],
              m[col * 4 + 3]),
    ];
  }
}

/// Row [row] of the three rows of four in [rows], times `(x, y, z, w)`.
double _dot(
        List<double> rows, int row, double x, double y, double z, double w) =>
    rows[row * 4] * x +
    rows[row * 4 + 1] * y +
    rows[row * 4 + 2] * z +
    rows[row * 4 + 3] * w;

/// The inverse of a three by three matrix, row by row, or null when there is
/// none: a layer seen exactly edge on.
List<double>? _invert3(List<double> m) {
  final a = m[4] * m[8] - m[5] * m[7];
  final b = m[5] * m[6] - m[3] * m[8];
  final c = m[3] * m[7] - m[4] * m[6];
  final det = m[0] * a + m[1] * b + m[2] * c;
  if (det.abs() < 1e-12) return null;
  final k = 1 / det;
  return [
    a * k,
    (m[2] * m[7] - m[1] * m[8]) * k,
    (m[1] * m[5] - m[2] * m[4]) * k,
    b * k,
    (m[0] * m[8] - m[2] * m[6]) * k,
    (m[2] * m[3] - m[0] * m[5]) * k,
    c * k,
    (m[1] * m[6] - m[0] * m[7]) * k,
    (m[0] * m[4] - m[1] * m[3]) * k,
  ];
}

/// [inv] times `(c, 1)`, divided through.
Offset _dehom(List<double> inv, Offset c) {
  final x = inv[0] * c.dx + inv[1] * c.dy + inv[2];
  final y = inv[3] * c.dx + inv[4] * c.dy + inv[5];
  final w = inv[6] * c.dx + inv[7] * c.dy + inv[8];
  final safe = w.abs() < 1e-12 ? 1e-12 : w;
  return Offset(x / safe, y / safe);
}

/// The pan-behind position a moved anchor implies, so the layer stays visually
/// fixed while its origin slides — a port of egui's
/// `app_state::pan_behind_position` (the maths `anchor_overlay` commits): the
/// anchor delta is scaled and rotated exactly as a layer point would be, then
/// added to the current position. [scaleXPercent]/[scaleYPercent] are
/// percentages; [rotationDegrees] is in degrees.
Offset panBehindPosition({
  required Offset oldAnchor,
  required Offset newAnchor,
  required Offset position,
  required double scaleXPercent,
  required double scaleYPercent,
  required double rotationDegrees,
}) {
  final sx = scaleXPercent / 100.0;
  final sy = scaleYPercent / 100.0;
  final rot = rotationDegrees * math.pi / 180.0;
  final sin = math.sin(rot);
  final cos = math.cos(rot);
  final dx = (newAnchor.dx - oldAnchor.dx) * sx;
  final dy = (newAnchor.dy - oldAnchor.dy) * sy;
  final rx = dx * cos - dy * sin;
  final ry = dx * sin + dy * cos;
  return Offset(position.dx + rx, position.dy + ry);
}
