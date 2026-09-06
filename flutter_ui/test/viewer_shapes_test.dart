// The shape tools' geometry, and the Pen's draft: what each
// tool draws between two corners, and how a path grows point by point.
//
// Pure arithmetic in layer space, so it is checked by arithmetic. The one thing
// every case shares is that the drag's two points are opposite corners of the
// shape's box *whichever way round they were dragged* — which is what makes the
// tools behave the same in all four directions, and is easy to get wrong.

import 'dart:math' as math;

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_shapes.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:uuid/uuid.dart';

void main() {
  List<(double, double)> positions(List<dynamic> vertices) =>
      [for (final v in vertices) (v.x as double, v.y as double)];

  group('Rectangle', () {
    test('its four corners are the drag\'s box', () {
      final path = shapePath(
        tool: ToolMode.shapeRectangle,
        from: (10, 20),
        to: (110, 70),
      );
      expect(positions(path), [(10.0, 20.0), (110.0, 20.0), (110.0, 70.0), (10.0, 70.0)]);
      expect(path.every((v) => v.tanOutX == 0 && v.tanOutY == 0), isTrue,
          reason: 'a rectangle is all corners');
    });

    test('dragging up and to the left gives the same rectangle', () {
      final down = shapePath(
        tool: ToolMode.shapeRectangle,
        from: (10, 20),
        to: (110, 70),
      );
      final up = shapePath(
        tool: ToolMode.shapeRectangle,
        from: (110, 70),
        to: (10, 20),
      );
      expect(positions(up).toSet(), positions(down).toSet());
    });

    test('Shift makes it square, keeping the drag\'s direction', () {
      final path = shapePath(
        tool: ToolMode.shapeRectangle,
        from: (100, 100),
        to: (40, 90),
        square: true,
      );
      // The wider axis wins (60 across against 10 up), and both go up-left.
      expect(positions(path).toSet(), {
        (40.0, 40.0),
        (100.0, 40.0),
        (100.0, 100.0),
        (40.0, 100.0),
      });
    });
  });

  group('Ellipse', () {
    test('four vertices on the box\'s edge midpoints, with kappa handles', () {
      final path = shapePath(
        tool: ToolMode.shapeEllipse,
        from: (0, 0),
        to: (200, 100),
      );
      expect(positions(path), [
        (100.0, 0.0),
        (200.0, 50.0),
        (100.0, 100.0),
        (0.0, 50.0),
      ]);
      // The top vertex's handles run sideways, by kappa of the x radius.
      expect(path.first.tanOutX, closeTo(100 * kappa, 1e-9));
      expect(path.first.tanOutY, 0);
      expect(path.first.tanInX, closeTo(-100 * kappa, 1e-9));
    });

    test('Shift makes it a circle', () {
      final path = shapePath(
        tool: ToolMode.shapeEllipse,
        from: (0, 0),
        to: (200, 100),
        square: true,
      );
      final xs = [for (final v in path) v.x];
      final ys = [for (final v in path) v.y];
      final width = xs.reduce((a, b) => a > b ? a : b) -
          xs.reduce((a, b) => a < b ? a : b);
      final height = ys.reduce((a, b) => a > b ? a : b) -
          ys.reduce((a, b) => a < b ? a : b);
      expect(width, closeTo(height, 1e-9));
    });
  });

  group('Rounded rectangle', () {
    test('two vertices per corner, rounded by a quarter of the short side', () {
      final path = shapePath(
        tool: ToolMode.shapeRoundedRectangle,
        from: (0, 0),
        to: (200, 100),
      );
      expect(path.length, 8);
      // The short side is 100, so the radius is 25.
      expect(path[0].x, 25);
      expect(path[0].y, 0);
      expect(path[1].x, 175);
      // Every vertex sits on the box's edge, never outside it.
      for (final v in path) {
        expect(v.x, inInclusiveRange(0, 200));
        expect(v.y, inInclusiveRange(0, 100));
      }
    });
  });

  group('Star', () {
    test('ten vertices, alternating out and in, first point at the top', () {
      final path = shapePath(
        tool: ToolMode.shapeStar,
        from: (0, 0),
        to: (100, 100),
      );
      expect(path.length, starPoints * 2);
      expect(path.first.x, closeTo(50, 1e-9));
      expect(path.first.y, closeTo(0, 1e-9), reason: 'the top point');
      // Outer points reach the box; inner ones are 40% of the way out.
      final centre = 50.0;
      final outer = (path[0].x - centre).abs() + (path[0].y - centre).abs();
      final inner = (path[1].x - centre).abs() + (path[1].y - centre).abs();
      expect(inner, lessThan(outer));
    });
  });

  group('Polygon', () {
    test('a regular five-sided figure in the box, first point at the top', () {
      final path = shapePath(
        tool: ToolMode.shapePolygon,
        from: (0, 0),
        to: (100, 100),
      );
      expect(path.length, polygonSides);
      expect(path.first.x, closeTo(50, 1e-9));
      expect(path.first.y, closeTo(0, 1e-9), reason: 'the top point');
      // Every point sits on the ellipse inscribed in the box, so every one is
      // the same distance from the middle of a square box.
      for (final v in path) {
        final d = math.sqrt(math.pow(v.x - 50, 2) + math.pow(v.y - 50, 2));
        expect(d, closeTo(50, 1e-9));
      }
      expect(path.every((v) => v.tanOutX == 0 && v.tanOutY == 0), isTrue,
          reason: 'a polygon is all corners');
    });
  });

  /// The Pen's path builder. This gesture was briefly on the polygon
  /// tool; it is After Effects' pen, and it belongs to the Pen.
  group('The Pen\'s draft', () {
    test('a click adds a corner', () {
      final draft = const PathDraft().withCorner((10, 10));
      expect(draft.vertices.length, 1);
      expect(draft.vertices.single.tanOutX, 0);
      expect(draft.canClose, isFalse, reason: 'one point is not a shape');
    });

    test('three points make it closable', () {
      final draft = const PathDraft()
          .withCorner((0, 0))
          .withCorner((10, 0))
          .withCorner((10, 10));
      expect(draft.canClose, isTrue);
      expect(draft.first, (0.0, 0.0));
    });

    test('a click-drag mirrors the handles, so the curve runs through', () {
      final draft =
          const PathDraft().withBezier((50, 50), (70, 50));
      final v = draft.vertices.single;
      expect(v.tanOutX, 20);
      expect(v.tanOutY, 0);
      expect(v.tanInX, -20, reason: 'the reflection of the one being dragged');
      expect(v.tanInY, 0);
    });

    test('Alt breaks the pair, leaving the entering handle alone', () {
      final draft = const PathDraft()
          .withBezier((50, 50), (70, 30), independent: true);
      final v = draft.vertices.single;
      expect(v.tanOutX, 20);
      expect(v.tanOutY, -20);
      expect(v.tanInX, 0, reason: 'independent, so it did not follow');
      expect(v.tanInY, 0);
    });

    test('the last point can be taken back', () {
      final draft = const PathDraft()
          .withCorner((0, 0))
          .withCorner((10, 0))
          .withoutLast();
      expect(draft.vertices.length, 1);
      expect(const PathDraft().withoutLast().isEmpty, isTrue,
          reason: 'and taking back nothing is not an error');
    });
  });

  group('Closing the path', () {
    test('a click near the first point closes it, one far away does not', () {
      expect(
        withinClosingDistance((3, 4), (0, 0), screenScale: 1),
        isTrue,
      );
      expect(
        withinClosingDistance((30, 40), (0, 0), screenScale: 1),
        isFalse,
      );
    });

    test('the tolerance is in screen pixels, so it follows the magnification',
        () {
      // Zoomed right out, ten layer pixels is one screen pixel: still a click
      // on the point.
      expect(withinClosingDistance((100, 0), (0, 0), screenScale: 0.1), isTrue);
      // Zoomed right in, one layer pixel is ten screen ones: not a click on it.
      expect(withinClosingDistance((2, 0), (0, 0), screenScale: 10), isFalse);
    });
  });

  /// **What the Viewer's feather guide is drawn from**: half the soft
  /// edge's width at each point, and nothing at all when there is no width to
  /// show or the width is keyframed.
  group('The feather guide', () {
    BridgeMask masked({
      required BridgeScalar feather,
      List<BridgeScalar> perPoint = const [],
    }) =>
        BridgeMask(
          id: UuidValue.fromString(const Uuid().v4()),
          name: 'Mask',
          vertices: shapePath(
            tool: ToolMode.shapeRectangle,
            from: (0, 0),
            to: (100, 50),
          ),
          closed: true,
          inverted: false,
          opacity: const BridgeScalar.static_(100),
          mode: BridgeMaskMode.add,
          feather: feather,
          vertexFeather: perPoint,
          expansion: const BridgeScalar.static_(0),
          pathKeys: const [],
        );

    test('a hard-edged mask has no guide to draw', () {
      expect(stillHalfFeather(masked(feather: const BridgeScalar.static_(0))),
          isNull);
    });

    test('one width gives every point the same half', () {
      expect(stillHalfFeather(masked(feather: const BridgeScalar.static_(20))),
          [10.0, 10.0, 10.0, 10.0]);
    });

    test('per-point widths are halved point by point', () {
      expect(
        stillHalfFeather(masked(
          feather: const BridgeScalar.static_(20),
          perPoint: const [
            BridgeScalar.static_(0),
            BridgeScalar.static_(40),
            BridgeScalar.static_(4),
            BridgeScalar.static_(0),
          ],
        )),
        [0.0, 20.0, 2.0, 0.0],
      );
    });

    test('a keyframed width draws nothing, rather than a width that is not the '
        'one in the picture', () {
      expect(
        stillHalfFeather(masked(
          feather: const BridgeScalar.static_(20),
          perPoint: const [
            BridgeScalar.static_(0),
            BridgeScalar.keyframed([]),
            BridgeScalar.static_(4),
            BridgeScalar.static_(0),
          ],
        )),
        isNull,
      );
    });
  });
}
