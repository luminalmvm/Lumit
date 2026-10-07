// A shape layer's size: the box its art fills.
//
// This is the one number a shape layer shares with the engine — the renderer
// sizes the raster with `shape::ShapeItem::bounds` and the Viewer draws the
// wireframe from this. If the two disagree, the box on screen is not the box
// the picture was drawn into, so both follow the same rule: the **control
// points** bound the curve, because a cubic never leaves its own control hull.

import 'dart:ui' show Size;

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/layer_bounds.dart';
import 'package:uuid/uuid.dart';

void main() {
  BridgeVertex corner(double x, double y) =>
      BridgeVertex(x: x, y: y, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0);

  BridgeShapeItem item(
    List<BridgeVertex> vertices, {
    double strokeWidth = 0,
    bool stroked = false,
    double copies = 1,
    double stepX = 0,
    double copyOffset = 0,
    double offset = 0,
  }) =>
      BridgeShapeItem(
        id: UuidValue.fromString(const Uuid().v4()),
        name: 'Rectangle',
        vertices: vertices,
        closed: true,
        fill: const BridgeColourRgba(r: 1, g: 1, b: 1, a: 1),
        stroke:
            stroked ? const BridgeColourRgba(r: 0, g: 0, b: 0, a: 1) : null,
        strokeWidth: strokeWidth,
        opacity: 100,
        trimStart: const BridgeScalar.static_(0),
        trimEnd: const BridgeScalar.static_(100),
        trimOffset: const BridgeScalar.static_(0),
        dashes: const [],
        dashOffset: const BridgeScalar.static_(0),
        gradient: 0,
        gradientColour: null,
        gradientStartX: const BridgeScalar.static_(0),
        gradientStartY: const BridgeScalar.static_(0),
        gradientEndX: const BridgeScalar.static_(0),
        gradientEndY: const BridgeScalar.static_(0),
        combine: 0,
        pathKeys: const [],
        offsetAmount: BridgeScalar.static_(offset),
        repeatCopies: BridgeScalar.static_(copies),
        repeatOffset: BridgeScalar.static_(copyOffset),
        repeatAnchorX: const BridgeScalar.static_(0),
        repeatAnchorY: const BridgeScalar.static_(0),
        repeatPositionX: BridgeScalar.static_(stepX),
        repeatPositionY: const BridgeScalar.static_(0),
        repeatRotation: const BridgeScalar.static_(0),
        repeatScale: const BridgeScalar.static_(100),
        repeatStartOpacity: const BridgeScalar.static_(100),
        repeatEndOpacity: const BridgeScalar.static_(100),
      );

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
}
