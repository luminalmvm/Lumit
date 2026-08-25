// The Rotation tool's pointer (K-219): which way the curved arrow leans, and
// how tight its curve is.
//
// Both are pure functions of where the pointer is over the layer, so both are
// checked here. What the drag then commits is `rotationForDrag`, which the
// gizmo's own tests already pin — this is the part that is new.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_gizmo.dart';
import 'package:lumit_flutter/panels/viewer_layer_map.dart';
import 'package:lumit_flutter/panels/viewer_rotate.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

void main() {
  /// A 200×100 layer at (300, 200), anchored on its own middle, drawn 1:1.
  LayerBox box({double rotation = 0}) => LayerBox(
        layer: LayerReference(
          internalprojectId: UuidValue.fromString(const Uuid().v4()),
          internalcompId: UuidValue.fromString(const Uuid().v4()),
          internallayerId: UuidValue.fromString(const Uuid().v4()),
        ),
        id: UuidValue.fromString(const Uuid().v4()),
        map: ViewerLayerMap.of(
          positionX: 300,
          positionY: 200,
          anchorX: 100,
          anchorY: 50,
          scaleXPercent: 100,
          scaleYPercent: 100,
          rotationDegrees: rotation,
          origin: Offset.zero,
          viewScale: 1,
        ),
        bounds: const Size(200, 100),
        draggable: true,
        scalable: true,
        rotationDegrees: rotation,
      );

  group('Which way the pointer leans', () {
    test('it faces away from the anchor, so the curve goes round it', () {
      final b = box();
      // Straight out to the right of the anchor at (300, 200).
      final right = rotateCursorFor(pointer: const Offset(500, 200), box: b);
      expect(right.angle, closeTo(0, 1e-9));

      // Straight down.
      final down = rotateCursorFor(pointer: const Offset(300, 400), box: b);
      expect(down.angle, closeTo(math.pi / 2, 1e-9));

      // Up and to the left: it settles on the layer's own top-left corner, so
      // it faces where that corner actually is on screen — which on a 2:1 box
      // is not 45°, and should not be (K-230).
      final upLeft = rotateCursorFor(pointer: const Offset(200, 100), box: b);
      final corner = b.map.toScreen(0, 0) - b.map.toScreen(100, 50);
      expect(upLeft.angle, closeTo(math.atan2(corner.dy, corner.dx), 1e-9));
    });

    test('it takes one of eight positions and nothing between them', () {
      final b = box();
      // A sweep right round the layer: every angle it produces has to be one
      // the box's own eight compass points can account for.
      final settled = <double>{};
      for (var degrees = 0; degrees < 360; degrees += 3) {
        final radians = degrees * math.pi / 180;
        final shape = rotateCursorFor(
          pointer: Offset(300 + 400 * math.cos(radians),
              200 + 400 * math.sin(radians)),
          box: b,
        );
        settled.add((shape.angle * 1e6).roundToDouble());
      }
      expect(settled.length, rotateCursorPositions);
    });

    test('the pointer exactly on the anchor has no direction to take, and does'
        ' not produce a NaN', () {
      final shape = rotateCursorFor(pointer: const Offset(300, 200), box: box());
      expect(shape.angle.isFinite, isTrue);
      expect(shape.sweep, rotateCursorEdgeSweep);
    });

    test('with nothing to lean round it points up and keeps the edge shape',
        () {
      final shape = rotateCursorFor(pointer: const Offset(10, 10), box: null);
      expect(shape.angle, closeTo(-math.pi / 2, 1e-9));
      expect(shape.sweep, rotateCursorEdgeSweep);
    });
  });

  group('How tight the curve is', () {
    test('square out from an edge is the shallow arc', () {
      final b = box();
      // Straight right of the anchor: the vertical is dead centre, so this is
      // as edge-like as a position gets.
      final edge = rotateCursorFor(pointer: const Offset(520, 200), box: b);
      expect(edge.sweep, closeTo(rotateCursorEdgeSweep, 1e-9));
    });

    test('out towards a corner is the tight arc', () {
      final b = box();
      // The layer's own corner is at (400, 250) — 100 across and 50 down from
      // the anchor, so a point on that diagonal is fully corner-ish.
      final corner = rotateCursorFor(pointer: const Offset(500, 300), box: b);
      expect(corner.sweep, closeTo(rotateCursorCornerSweep, 1e-9));
    });

    // It used to slide between the two, which was true to the geometry and
    // harder to read than eight settled shapes (K-230).
    test('between the two it is still one of the two', () {
      final b = box();
      final between = rotateCursorFor(pointer: const Offset(500, 225), box: b);
      expect(
        between.sweep,
        anyOf(rotateCursorEdgeSweep, rotateCursorCornerSweep),
      );
    });

    test('a corner stays a corner when the layer is turned', () {
      // The same corner of the *layer*, now that the layer is on its side: the
      // measurement is in layer space, so the shape must not change.
      final upright = rotateCursorFor(
        pointer: const Offset(500, 300),
        box: box(),
      );
      final turned = box(rotation: 90);
      // Rotating the point (200, 100) about the anchor by 90° puts it at
      // (100, 400) relative to the same anchor at (300, 200).
      final onSide = rotateCursorFor(
        pointer: const Offset(200, 400),
        box: turned,
      );
      expect(onSide.sweep, closeTo(upright.sweep, 1e-6));
    });
  });
}
