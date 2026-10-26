// A shape layer's size: the box its art fills.
//
// This is the one number a shape layer shares with the engine — the renderer
// sizes the raster with `shape::ShapeItem::bounds` and the Viewer draws the
// wireframe from this. If the two disagree, the box on screen is not the box
// the picture was drawn into, so both follow the same rule: the **control
// points** bound the curve, because a cubic never leaves its own control hull.

import 'dart:ui' show Size;

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/shape.dart';
import 'package:lumit_flutter/state/layer_bounds.dart';
import 'package:uuid/uuid.dart';

void main() {
  BridgeVertex corner(double x, double y) =>
      BridgeVertex(x: x, y: y, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0);

  BridgeShapeEntry item(
    List<BridgeVertex> vertices, {
    double strokeWidth = 0,
    bool stroked = false,
  }) =>
      BridgeShapeEntry(
          depth: 0,
          kind: BridgeShapeEntryKind.item(BridgeShapeItem(
        id: UuidValue.fromString(const Uuid().v4()),
        name: 'Rectangle',
        vertices: vertices,
        closed: true,
        fill: const BridgeColour(
            r: BridgeScalar.static_(1),
            g: BridgeScalar.static_(1),
            b: BridgeScalar.static_(1),
            a: BridgeScalar.static_(1)),
        stroke: stroked
            ? const BridgeColour(
                r: BridgeScalar.static_(0),
                g: BridgeScalar.static_(0),
                b: BridgeScalar.static_(0),
                a: BridgeScalar.static_(1))
            : null,
        strokeWidth: BridgeScalar.static_(strokeWidth),
        opacity: const BridgeScalar.static_(100),
        dashes: const [],
        dashOffset: const BridgeScalar.static_(0),
        gradient: 0,
        gradientColour: null,
        gradientStartX: const BridgeScalar.static_(0),
        gradientStartY: const BridgeScalar.static_(0),
        gradientEndX: const BridgeScalar.static_(0),
        gradientEndY: const BridgeScalar.static_(0),
        gradientStartOpacity: const BridgeScalar.static_(100),
        gradientEndOpacity: const BridgeScalar.static_(100),
        gradientStops: const [],
        combine: 0,
        pathKeys: const [],
        taper: null,
      )));

  /// A repeater entry stepping [copies] times by [stepX].
  BridgeShapeEntry repeater(double copies, double stepX) => BridgeShapeEntry(
      depth: 0,
      kind: BridgeShapeEntryKind.modifier(BridgeShapeModifier(
        id: UuidValue.fromString(const Uuid().v4()),
        name: 'Repeater',
        kind: BridgeShapeModifierKind.repeat(BridgeShapeRepeater(
          copies: BridgeScalar.static_(copies),
          offset: const BridgeScalar.static_(0),
          anchorX: const BridgeScalar.static_(0),
          anchorY: const BridgeScalar.static_(0),
          positionX: BridgeScalar.static_(stepX),
          positionY: const BridgeScalar.static_(0),
          rotation: const BridgeScalar.static_(0),
          scale: const BridgeScalar.static_(100),
          startOpacity: const BridgeScalar.static_(100),
          endOpacity: const BridgeScalar.static_(100),
        )),
      )));

  test('the art\'s own box is the layer\'s size', () {
    final size = shapeContentsBounds([
      item([corner(10, 20), corner(40, 20), corner(40, 60), corner(10, 60)]),
    ]);
    expect(size, const Size(30, 40));
  });

  test('a handle reaching outside the vertices is inside the box', () {
    final size = shapeContentsBounds([
      item([
        const BridgeVertex(
            x: 0, y: 0, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: -20),
        corner(10, 0),
        corner(10, 10),
        corner(0, 10),
      ]),
    ]);
    expect(size!.height, 30, reason: 'the handle reaches 20 above the art');
  });

  test('a repeater below the art grows the box, and two of them multiply',
      () {
    final square = [corner(0, 0), corner(10, 0), corner(10, 10), corner(0, 10)];
    expect(shapeContentsBounds([item(square), repeater(3, 20)]),
        const Size(50, 10));
    // The second repeater repeats the copies of the first: a grid.
    expect(
        shapeContentsBounds([item(square), repeater(3, 20), repeater(2, 0)]),
        const Size(50, 10));
    // Above the art, it acts on nothing.
    expect(shapeContentsBounds([repeater(3, 20), item(square)]),
        const Size(10, 10));
  });
}
