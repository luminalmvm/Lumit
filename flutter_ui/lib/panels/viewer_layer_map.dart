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

/// A scale factor that can be flipped but never vanishes (K-230).
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

  const ViewerLayerMap({
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
      );

  /// The same map with the layer's pivot moved to ([anchorX], [anchorY]) and
  /// its position set to [position] — the pair a pan-behind writes together.
  ///
  /// What a pivot drag in flight looks like (K-235): the anchor mark moves and
  /// the picture, box and all, deliberately does not.
  ViewerLayerMap pivotedAt(
    double anchorX,
    double anchorY, {
    required Offset position,
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
      );

  /// Layer space → screen (local) coordinate.
  Offset toScreen(double x, double y) {
    final dx = (x - ax) * sx;
    final dy = (y - ay) * sy;
    final rx = dx * cos - dy * sin;
    final ry = dx * sin + dy * cos;
    return Offset(
      origin.dx + (px + rx) * viewScale,
      origin.dy + (py + ry) * viewScale,
    );
  }

  /// Screen (local) → layer space coordinate (the inverse of [toScreen]).
  Offset layerOf(Offset pos) {
    final cx = (pos.dx - origin.dx) / viewScale;
    final cy = (pos.dy - origin.dy) / viewScale;
    final dx = cx - px;
    final dy = cy - py;
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
    final anchorScreen = toScreen(ax, ay);
    final ex = (pointer.dx - anchorScreen.dx) / viewScale;
    final ey = (pointer.dy - anchorScreen.dy) / viewScale;
    // Un-rotate the screen delta into the layer's un-scaled frame.
    final ux = ex * cos + ey * sin;
    final uy = -ex * sin + ey * cos;
    final newSx = dxFromAnchor.abs() < 1e-9 ? sx : ux / dxFromAnchor;
    final newSy = dyFromAnchor.abs() < 1e-9 ? sy : uy / dyFromAnchor;
    return (newSx * 100.0, newSy * 100.0);
  }
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
