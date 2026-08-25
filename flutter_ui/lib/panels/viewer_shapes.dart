// The shape tools and the Pen: the paths they draw, in the layer's own
// coordinates (K-222, K-223, docs/07 §1.7, §2.3.1).
//
// **In plain terms.** A mask is a shape drawn on a layer that decides which of
// its pixels show. The shape tools draw one: rectangle, rounded rectangle,
// ellipse, polygon and star all drag out corner to corner. The **Pen** is the
// odd one out — it builds a path point by point ([PathDraft]) — and it lives
// here too because what it builds is the same kind of path. This file is the
// *geometry*: given two corners, what vertices does an ellipse have? It is pure,
// so it is tested by arithmetic rather than by dragging.
//
// **Everything here is in layer space.** A mask travels with its layer's
// transform: move the layer and the mask moves with it, because the path is
// written in the same coordinates the layer's own pixels use. So the tool takes
// the pointer's screen position, runs it *backwards* through the layer's map
// (the same inverse the wireframe hit-tests with), and stores what comes out.
// Nothing here knows about the screen at all.
//
// The vertex shape is the engine's, carried across the bridge unchanged: a
// position and two tangent handles, each an offset from the position. A corner
// is a vertex whose handles are both zero.

import 'dart:math' as math;
import 'dart:ui' show Offset, Path;

import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:uuid/uuid.dart';

/// The circle-from-cubics constant: how far a bezier handle reaches, as a
/// fraction of the radius, for four cubics to approximate a circle. The engine
/// uses the same number in `mask::Mask::ellipse`, and the two must agree or a
/// mask drawn here would not be the mask the engine would have made.
const double kappa = 0.5522847498307934;

/// How round a rounded rectangle's corners are: a quarter of its shorter side,
/// which is Lumit's default and is what makes the tool visibly different from
/// the plain rectangle at any size.
const double roundedCornerFraction = 0.25;

/// How many points a star has, and how deep its notches go.
const int starPoints = 5;
const double starInnerFraction = 0.4;

/// How many sides a polygon has. After Effects' own default, and the same count
/// as the star's points so the two tools read as a pair.
const int polygonSides = 5;

/// A corner vertex — no curvature either side.
BridgeVertex shapeCorner(double x, double y) => BridgeVertex(
      x: x,
      y: y,
      tanInX: 0,
      tanInY: 0,
      tanOutX: 0,
      tanOutY: 0,
    );

/// [to], moved so that the rectangle from [from] keeps square proportions —
/// Shift's rule for every shape tool.
///
/// The larger of the two sides wins, and each axis keeps its own direction, so
/// dragging up and to the left gives a square up and to the left rather than
/// flipping the shape through the start point.
({double dx, double dy}) squareExtent(double dx, double dy) {
  final side = math.max(dx.abs(), dy.abs());
  return (
    dx: dx.isNegative ? -side : side,
    dy: dy.isNegative ? -side : side,
  );
}

/// The path a drag from [from] to [to] draws for [tool], in layer space.
///
/// The two points are opposite corners of the shape's bounding box — whichever
/// way round they were dragged — which is what makes "start here, end there"
/// mean the same thing in all four directions. [square] is Shift.
///
/// The Pen is not here: it does not drag out a shape, it builds one point by
/// point (see [PathDraft]).
List<BridgeVertex> shapePath({
  required ToolMode tool,
  required (double, double) from,
  required (double, double) to,
  bool square = false,
}) {
  var dx = to.$1 - from.$1;
  var dy = to.$2 - from.$2;
  if (square) {
    final e = squareExtent(dx, dy);
    dx = e.dx;
    dy = e.dy;
  }
  // Normalised: left/top are the smaller coordinates whichever way the drag
  // went, so every shape below can be written the easy way round.
  final left = math.min(from.$1, from.$1 + dx);
  final top = math.min(from.$2, from.$2 + dy);
  final w = dx.abs();
  final h = dy.abs();
  final right = left + w;
  final bottom = top + h;

  switch (tool) {
    case ToolMode.shapeRectangle:
      return [
        shapeCorner(left, top),
        shapeCorner(right, top),
        shapeCorner(right, bottom),
        shapeCorner(left, bottom),
      ];

    case ToolMode.shapeRoundedRectangle:
      // Two vertices per corner, with handles pulling towards the corner
      // itself: the standard way to round a rectangle with cubics.
      final r = math.min(w, h) * roundedCornerFraction;
      final k = r * kappa;
      return [
        BridgeVertex(
            x: left + r, y: top, tanInX: -k, tanInY: 0, tanOutX: 0, tanOutY: 0),
        BridgeVertex(
            x: right - r, y: top, tanInX: 0, tanInY: 0, tanOutX: k, tanOutY: 0),
        BridgeVertex(
            x: right,
            y: top + r,
            tanInX: 0,
            tanInY: -k,
            tanOutX: 0,
            tanOutY: 0),
        BridgeVertex(
            x: right,
            y: bottom - r,
            tanInX: 0,
            tanInY: 0,
            tanOutX: 0,
            tanOutY: k),
        BridgeVertex(
            x: right - r,
            y: bottom,
            tanInX: k,
            tanInY: 0,
            tanOutX: 0,
            tanOutY: 0),
        BridgeVertex(
            x: left + r,
            y: bottom,
            tanInX: 0,
            tanInY: 0,
            tanOutX: -k,
            tanOutY: 0),
        BridgeVertex(
            x: left,
            y: bottom - r,
            tanInX: 0,
            tanInY: k,
            tanOutX: 0,
            tanOutY: 0),
        BridgeVertex(
            x: left, y: top + r, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: -k),
      ];

    case ToolMode.shapeEllipse:
      final rx = w / 2;
      final ry = h / 2;
      final cx = left + rx;
      final cy = top + ry;
      return [
        BridgeVertex(
            x: cx,
            y: cy - ry,
            tanInX: -rx * kappa,
            tanInY: 0,
            tanOutX: rx * kappa,
            tanOutY: 0),
        BridgeVertex(
            x: cx + rx,
            y: cy,
            tanInX: 0,
            tanInY: -ry * kappa,
            tanOutX: 0,
            tanOutY: ry * kappa),
        BridgeVertex(
            x: cx,
            y: cy + ry,
            tanInX: rx * kappa,
            tanInY: 0,
            tanOutX: -rx * kappa,
            tanOutY: 0),
        BridgeVertex(
            x: cx - rx,
            y: cy,
            tanInX: 0,
            tanInY: ry * kappa,
            tanOutX: 0,
            tanOutY: -ry * kappa),
      ];

    case ToolMode.shapeStar:
      // Inscribed in the box, first point at the top — the engine's own star
      // (`mask::Mask::star`), so the two agree about what a star is.
      final rx = w / 2;
      final ry = h / 2;
      final cx = left + rx;
      final cy = top + ry;
      return [
        for (var i = 0; i < starPoints * 2; i++)
          () {
            final outer = i.isEven;
            final f = outer ? 1.0 : starInnerFraction;
            final a = math.pi * i / starPoints - math.pi / 2;
            return shapeCorner(
              cx + rx * f * math.cos(a),
              cy + ry * f * math.sin(a),
            );
          }(),
      ];

    case ToolMode.shapePolygon:
      // A regular polygon inscribed in the box, first point at the top — the
      // star without its notches, and the same tool After Effects has. (The
      // *path-building* gesture that was briefly on this tool belongs to the
      // Pen, where After Effects puts it — K-223.)
      final rx = w / 2;
      final ry = h / 2;
      final cx = left + rx;
      final cy = top + ry;
      return [
        for (var i = 0; i < polygonSides; i++)
          () {
            final a = 2 * math.pi * i / polygonSides - math.pi / 2;
            return shapeCorner(cx + rx * math.cos(a), cy + ry * math.sin(a));
          }(),
      ];

    // Not a shape tool: the Pen builds its path point by point (see [PathDraft]).
    case _:
      return const [];
  }
}

/// One cubic path through [count] vertices, on whichever screen mapping the
/// caller draws with — the walk every path outline and preview shares (K-224,
/// K-237). The callbacks answer, for vertex `i`, where it sits and where its
/// two handles reach, so a caller can fold in nudges or an art offset without
/// a second copy of the cubic walk.
Path bezierPath({
  required int count,
  required Offset Function(int i) at,
  required Offset Function(int i) tangentOut,
  required Offset Function(int i) tangentIn,
  required bool closed,
}) {
  final path = Path()..moveTo(at(0).dx, at(0).dy);
  for (var i = 1; i < count; i++) {
    final c1 = tangentOut(i - 1);
    final c2 = tangentIn(i);
    path.cubicTo(c1.dx, c1.dy, c2.dx, c2.dy, at(i).dx, at(i).dy);
  }
  if (closed && count > 2) {
    final c1 = tangentOut(count - 1);
    final c2 = tangentIn(0);
    path.cubicTo(c1.dx, c1.dy, c2.dx, c2.dy, at(0).dx, at(0).dy);
    path.close();
  }
  return path;
}

/// [mask] showing [vertices] instead of its own — the shape an animated path
/// is really at (K-342). Everything else is the mask untouched, so this is
/// only ever a *view* of it and never something written back.
BridgeMask maskWithVertices(BridgeMask mask, List<BridgeVertex> vertices) =>
    BridgeMask(
      id: mask.id,
      name: mask.name,
      vertices: vertices,
      closed: mask.closed,
      inverted: mask.inverted,
      opacity: mask.opacity,
      mode: mask.mode,
      feather: mask.feather,
      expansion: mask.expansion,
      pathKeys: mask.pathKeys,
    );

/// A mask ready to send, from a path and a name.
BridgeMask shapeMask({
  required List<BridgeVertex> vertices,
  required String name,
  bool closed = true,
}) =>
    BridgeMask(
      id: UuidValue.fromString(const Uuid().v4()),
      name: name,
      vertices: vertices,
      closed: closed,
      inverted: false,
      opacity: const BridgeScalar.static_(100),
      mode: BridgeMaskMode.add,
      feather: const BridgeScalar.static_(0),
      expansion: const BridgeScalar.static_(0),
      // A shape just drawn has no keys; a mask being edited keeps its own,
      // which the engine patches back.
      pathKeys: const [],
    );

/// What a shape drawn by [tool] is called. Named for the shape rather than
/// numbered, because a layer's list reads better as "Ellipse, Star" than as
/// "Mask 1, Mask 2" — and the Timeline lets either be renamed.
String shapeMaskName(ToolMode tool) => switch (tool) {
      ToolMode.shapeRectangle => l10n.toolShapeRectangle,
      ToolMode.shapeRoundedRectangle => l10n.toolShapeRoundedRectangle,
      ToolMode.shapeEllipse => l10n.toolShapeEllipse,
      ToolMode.shapeStar => l10n.toolShapeStar,
      ToolMode.shapePolygon => l10n.toolShapePolygon,
      ToolMode.pen => l10n.shapePath,
      _ => l10n.shapeMask,
    };

/// What a *mask* drawn by [tool] on a layer that already has [existing] of them
/// is called.
///
/// The shapes keep their shape names, as above. **The Pen numbers instead:**
/// every path it draws is a path, so calling them all "Path" says nothing about
/// which row is which, where the number does. [existing] is the count the
/// layer already carries, so the first is "Mask 1"; counting rather than
/// reading the highest name means a deleted mask's number comes round again,
/// which is what makes this a *default* — the row can be renamed at once.
///
/// Separate from [shapeMaskName] because the Pen also names shape *layers*,
/// and a shape layer is not a mask.
String maskName(ToolMode tool, int existing) => tool == ToolMode.pen
    ? l10n.maskNumbered(existing + 1)
    : shapeMaskName(tool);

/// A path being drawn with the **Pen** (K-223): the vertices placed so far.
///
/// **The gesture this models.** A click places a corner. A click *and drag*
/// places a vertex and pulls a pair of bezier handles out of it, mirrored so
/// the curve runs smoothly through — the handle you drag is the one leaving the
/// vertex, and the one entering it is its reflection. Holding `Alt` during that
/// drag breaks the pair: the entering handle stays where it was and only the
/// leaving one follows, which is how a corner with one curved side is made.
/// Clicking the first vertex again closes the path, and that is what commits
/// it.
///
/// Immutable-ish by design: every gesture returns a new draft rather than
/// mutating one, so the widget holds a value and the tests need no widget.
class PathDraft {
  final List<BridgeVertex> vertices;

  const PathDraft({this.vertices = const []});

  bool get isEmpty => vertices.isEmpty;

  /// A path can only close once it is a shape: two points and a line back is
  /// not one.
  bool get canClose => vertices.length >= 3;

  /// The first vertex's position, which is the point a click must land on to
  /// close the path.
  (double, double)? get first =>
      vertices.isEmpty ? null : (vertices.first.x, vertices.first.y);

  /// With a corner placed at [at].
  PathDraft withCorner((double, double) at) => PathDraft(
        vertices: [...vertices, shapeCorner(at.$1, at.$2)],
      );

  /// With a vertex at [at] whose leaving handle reaches [handle].
  ///
  /// [independent] is `Alt`: the entering handle keeps whatever it had (nothing,
  /// for a vertex being born) instead of mirroring the leaving one. Mirrored is
  /// the default because a smooth curve is what a dragged vertex is *for*.
  PathDraft withBezier(
    (double, double) at,
    (double, double) handle, {
    bool independent = false,
    BridgeVertex? previous,
  }) {
    final outX = handle.$1 - at.$1;
    final outY = handle.$2 - at.$2;
    final vertex = BridgeVertex(
      x: at.$1,
      y: at.$2,
      tanInX: independent ? (previous?.tanInX ?? 0) : -outX,
      tanInY: independent ? (previous?.tanInY ?? 0) : -outY,
      tanOutX: outX,
      tanOutY: outY,
    );
    return PathDraft(vertices: [...vertices, vertex]);
  }

  /// Without its last vertex — the undo inside the gesture.
  PathDraft withoutLast() => vertices.isEmpty
      ? this
      : PathDraft(vertices: vertices.sublist(0, vertices.length - 1));
}

/// Whether [at] is close enough to [target] to count as clicking it, measured
/// in layer pixels at the given magnification.
///
/// [screenScale] converts layer pixels to screen ones, so the tolerance is a
/// fixed number of *screen* pixels however far the picture is zoomed — the same
/// rule the anchor's snapping follows (K-220).
bool withinClosingDistance(
  (double, double) at,
  (double, double) target, {
  required double screenScale,
  double screenPixels = 10,
}) {
  final dx = (at.$1 - target.$1) * screenScale;
  final dy = (at.$2 - target.$2) * screenScale;
  return math.sqrt(dx * dx + dy * dy) <= screenPixels;
}
