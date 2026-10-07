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
import 'package:lumit_flutter/state/tools.dart';

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
  });

  group('Closing the path', () {
    test('the tolerance is in screen pixels, so it follows the magnification',
        () {
      // Zoomed right out, ten layer pixels is one screen pixel: still a click
      // on the point.
      expect(withinClosingDistance((100, 0), (0, 0), screenScale: 0.1), isTrue);
      // Zoomed right in, one layer pixel is ten screen ones: not a click on it.
      expect(withinClosingDistance((2, 0), (0, 0), screenScale: 10), isFalse);
    });
  });
}
