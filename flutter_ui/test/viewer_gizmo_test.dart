// The Viewer gizmo's arithmetic: which layer a point is inside, what a
// marquee catches, where the handles sit once a layer is turned, and what a
// handle drag means.
//
// All of it is pure, so all of it is checked here against hand-computed cases
// rather than by dragging in a widget tree — the same reasoning as
// viewer_layer_map.dart, whose maths these build on.

import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/panels/viewer_gizmo.dart';
import 'package:lumit_flutter/panels/viewer_layer_map.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

void main() {
  /// A layer of [size] sitting at [at] in a comp drawn 1:1 from the origin,
  /// anchored on its own middle — the arrangement a placed clip gets.
  LayerBox box({
    Size size = const Size(200, 100),
    Offset at = const Offset(300, 200),
    double scale = 100,
    double rotation = 0,
    Offset origin = Offset.zero,
    double viewScale = 1,
    List<BridgeMask> masks = const [],
    List<BridgeShapeItem> shapeContents = const [],
    Offset artOrigin = Offset.zero,
    BridgeMotionPath? motionPath,
  }) =>
      LayerBox(
        layer: LayerReference(
          internalprojectId: UuidValue.fromString(const Uuid().v4()),
          internalcompId: UuidValue.fromString(const Uuid().v4()),
          internallayerId: UuidValue.fromString(const Uuid().v4()),
        ),
        id: UuidValue.fromString(const Uuid().v4()),
        map: ViewerLayerMap.of(
          positionX: at.dx,
          positionY: at.dy,
          anchorX: size.width / 2,
          anchorY: size.height / 2,
          scaleXPercent: scale,
          scaleYPercent: scale,
          rotationDegrees: rotation,
          origin: origin,
          viewScale: viewScale,
        ),
        bounds: size,
        draggable: true,
        scalable: true,
        rotationDegrees: rotation,
        masks: masks,
        shapeContents: shapeContents,
        artOrigin: artOrigin,
        motionPath: motionPath,
      );

  /// A path from (100, 200) to (400, 500) in comp pixels over three frames,
  /// with a key at each end — the shape the engine hands back for a position
  /// keyed on both axes.
  BridgeMotionPath path() => BridgeMotionPath(
        firstFrame: 0,
        samples: Float64List.fromList([100, 200, 200, 300, 300, 400, 400, 500]),
        keys: [
          for (final (frame, x, y) in [(0, 100.0, 200.0), (3, 400.0, 500.0)])
            BridgeMotionKey(
              time: BridgeRational(num: frame, den: 30),
              frame: frame,
              x: x,
              y: y,
              xIndex: frame == 0 ? 0 : 1,
              yIndex: frame == 0 ? 0 : 1,
              handleIn: null,
              handleOut: null,
            ),
        ],
      );

  /// A square mask in the layer's own coordinates, all corners.
  BridgeMask squareMask({
    double left = 20,
    double top = 20,
    double side = 60,
  }) =>
      BridgeMask(
        id: UuidValue.fromString(const Uuid().v4()),
        name: 'Rectangle',
        vertices: [
          for (final (x, y) in [
            (left, top),
            (left + side, top),
            (left + side, top + side),
            (left, top + side),
          ])
            BridgeVertex(
                x: x, y: y, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
        ],
        closed: true,
        inverted: false,
        opacity: const BridgeScalar.static_(100),
        mode: BridgeMaskMode.add,
        feather: const BridgeScalar.static_(0),
        vertexFeather: const [],
        expansion: const BridgeScalar.static_(0),
        pathKeys: const [],
      );

  /// The same square, as a shape layer's own art rather than a mask. The two
  /// hold the same path type, which is the whole reason one set of helpers can
  /// serve both.
  BridgeShapeItem squareShape({
    double left = 20,
    double top = 20,
    double side = 60,
  }) =>
      BridgeShapeItem(
        id: UuidValue.fromString(const Uuid().v4()),
        name: 'Rectangle',
        vertices: [
          for (final (x, y) in [
            (left, top),
            (left + side, top),
            (left + side, top + side),
            (left, top + side),
          ])
            BridgeVertex(
                x: x, y: y, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
        ],
        closed: true,
        fill: null,
        stroke: null,
        strokeWidth: 0,
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
        offsetAmount: const BridgeScalar.static_(0),
        repeatCopies: const BridgeScalar.static_(1),
        repeatOffset: const BridgeScalar.static_(0),
        repeatAnchorX: const BridgeScalar.static_(0),
        repeatAnchorY: const BridgeScalar.static_(0),
        repeatPositionX: const BridgeScalar.static_(0),
        repeatPositionY: const BridgeScalar.static_(0),
        repeatRotation: const BridgeScalar.static_(0),
        repeatScale: const BridgeScalar.static_(100),
        repeatStartOpacity: const BridgeScalar.static_(100),
        repeatEndOpacity: const BridgeScalar.static_(100),
      );

  group('A motion path (docs/07 §2.4)', () {
    test('is drawn in comp pixels through the picture\'s placement alone', () {
      // Scaled and turned, which a comp-space point must ignore: Position is
      // where the anchor lands in the comp, not a point of the layer's own.
      final b = box(
        scale: 50,
        rotation: 45,
        origin: const Offset(10, 20),
        viewScale: 0.5,
        motionPath: path(),
      );
      final points = motionPathScreen(b);
      expect(points, hasLength(4));
      expect(points.first, const Offset(10 + 50, 20 + 100));
      expect(points.last, const Offset(10 + 200, 20 + 250));
      expect(motionPathScreen(box()), isEmpty,
          reason: 'a still position has nothing to draw');
    });

    test('moving a key writes that key on the axes keyed there, and no other',
        () {
      BridgeKeyframe key(int frame, double value) => BridgeKeyframe(
            time: BridgeRational(num: frame, den: 30),
            value: value,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          );
      final tf = BridgeTransform(
        anchorX: const BridgeScalar.static_(0),
        anchorY: const BridgeScalar.static_(0),
        positionX: BridgeScalar.keyframed([key(0, 100), key(3, 400)]),
        // y is still: a key dot on the path moves x only.
        positionY: const BridgeScalar.static_(200),
        positionZ: const BridgeScalar.static_(0),
        scaleX: const BridgeScalar.static_(100),
        scaleY: const BridgeScalar.static_(100),
        rotation: const BridgeScalar.static_(0),
        rotationX: const BridgeScalar.static_(0),
        rotationY: const BridgeScalar.static_(0),
        opacity: const BridgeScalar.static_(100),
      );
      final last = path().keys.last;
      final moved = transformWithMotionKey(
        tf,
        BridgeMotionKey(
          time: last.time,
          frame: last.frame,
          x: last.x,
          y: last.y,
          xIndex: 1,
          yIndex: null,
          handleIn: null,
          handleOut: null,
        ),
        const Offset(450, 999),
      );
      final x = moved.positionX as BridgeScalar_Keyframed;
      expect(x.field0.map((k) => k.value), [100, 450]);
      expect(x.field0[1].time, last.time, reason: 'the key keeps its time');
      expect(moved.positionY, tf.positionY,
          reason: 'an axis with no key there is left alone');
    });
  });

  group('What a point is inside', () {
    test('a rotated layer is tested in its own frame, not on screen', () {
      // Quarter-turned, the 200×100 layer occupies a 100×200 patch of screen.
      final b = box(rotation: 90);
      expect(b.contains(const Offset(300, 290)), isTrue,
          reason: '90 px down the screen is along the layer\'s own long axis');
      expect(b.contains(const Offset(390, 200)), isFalse,
          reason: 'and 90 px across it is outside the turned layer');
    });

    test('the topmost layer takes the click', () {
      final top = box(size: const Size(50, 50));
      final under = box();
      expect(layerAtPoint([top, under], const Offset(300, 200))?.id, top.id);
      // Beyond the small one, the big one below still answers.
      expect(layerAtPoint([top, under], const Offset(380, 200))?.id, under.id);
      expect(layerAtPoint([top, under], const Offset(600, 600)), isNull);
    });
  });

  group('What a marquee catches', () {
    test('only a box wholly inside it', () {
      final b = box();
      expect(b.insideRect(const Rect.fromLTRB(150, 100, 450, 300)), isTrue);
      expect(b.insideRect(const Rect.fromLTRB(150, 100, 350, 300)), isFalse,
          reason: 'the right-hand half is outside the sweep');
      expect(b.insideRect(const Rect.fromLTRB(0, 0, 10, 10)), isFalse);
    });
  });

  /// A mask's own points: with the Selection tool and the wireframes
  /// on, every vertex of every mask is a thing you can aim at, sweep up and
  /// drag. The arithmetic that decides *which* is here.
  group('A mask\'s points', () {
    test('a press near one names it, and one far away names nothing', () {
      final b = box(masks: [squareMask()]);
      final hit = pathPointAt([b], const Offset(223, 172));
      expect(hit, isNotNull);
      expect(hit!.index, 0);
      expect(hit.key, maskPointKey(b.id, b.masks.single.id, 0));
      expect(pathPointAt([b], const Offset(250, 200)), isNull,
          reason: 'the middle of the mask is not one of its points');
    });

    test('a sweep gathers every point inside it and no others', () {
      final b = box(masks: [squareMask()]);
      // The top edge only: the two points at y = 170, not the two at y = 230.
      final caught =
          pathPointsInRect([b], const Rect.fromLTRB(200, 150, 300, 200));
      expect(caught, {
        maskPointKey(b.id, b.masks.single.id, 0),
        maskPointKey(b.id, b.masks.single.id, 1),
      });
      expect(pathPointsInRect([b], const Rect.fromLTRB(0, 0, 10, 10)), isEmpty);
    });
  });

  /// A shape layer's own art is editable on the picture by the same gesture
  /// a mask's points take. The arithmetic is shared with masks; what these
  /// pin is that a shape item's points are found, named apart from a mask's,
  /// and swept up the same way.
  group("A shape layer's points", () {
    test('a shape point and a mask point are never the same point', () {
      final id = UuidValue.fromString(const Uuid().v4());
      final layer = UuidValue.fromString(const Uuid().v4());
      // Same layer, same path id, same index — and still two different points,
      // because one is written back with setMask and the other with
      // setShapeContents. Without the prefix a selection could not tell them
      // apart and one would be committed as the other.
      expect(maskPointKey(layer, id, 0), isNot(shapePointKey(layer, id, 0)));
    });

    /// **The art's coordinates are not the layer's pixels**. The engine
    /// draws a shape layer's picture as exactly its art's bounding box, so the
    /// layer's pixel (0, 0) is that box's corner. Drawing the points straight
    /// through the layer's map put every one of them a whole bounding box away
    /// from the art it belonged to — the box and the picture agreed, and only
    /// the points were somewhere else.
    test('are drawn from the art\'s own corner, not the path\'s numbers', () {
      // A 60x60 layer at (300, 200) anchored in its middle: its origin is at
      // (270, 170) on screen. The art is the same square, drawn at (120, 80) —
      // so its first point is the layer's origin and nowhere else.
      final b = box(
        size: const Size(60, 60),
        shapeContents: [squareShape(left: 120, top: 80, side: 60)],
        artOrigin: const Offset(120, 80),
      );
      final points = pathPointsOf(b);
      expect(points.first.at, const Offset(270, 170));
      expect(points[2].at, const Offset(330, 230),
          reason: 'the far corner of the art is the far corner of the box');
      expect(b.corners.first, points.first.at,
          reason: 'which is the whole point: the box and the points agree');
    });
  });

  group('Where the handles sit', () {
    test('a press near a handle finds it, and one far from any finds none', () {
      final b = box();
      expect(b.handleHit(const Offset(202, 152)), GizmoHandle.topLeft);
      expect(b.handleHit(const Offset(290, 180)), isNull,
          reason: 'open ground inside the layer is not a handle');
    });

    /// The anchor is a handle now, and it sits where a body drag begins — so
    /// it has to be *aimed at* rather than fallen into, or every drag of a
    /// layer would pan behind instead of moving it.
    test('the anchor is a handle, but only within a tight radius', () {
      final b = box();
      expect(b.handleHit(const Offset(300, 200)), GizmoHandle.anchor,
          reason: 'dead on the pivot');
      expect(b.handleHit(const Offset(304, 202)), GizmoHandle.anchor);
      expect(b.handleHit(const Offset(316, 200)), isNull,
          reason: 'a shade further out is a move, not a pan-behind');
    });
  });

  group('What a handle drag means', () {
    test('dragging a corner outward scales the layer up', () {
      final b = box();
      // The bottom-right corner sits at (400, 250) and the anchor at (300,
      // 200) — pulling the corner twice as far from the anchor doubles both.
      final (sx, sy) = scaleForGizmoHandle(
        box: b,
        handle: GizmoHandle.bottomRight,
        pointer: const Offset(500, 300),
        uniform: false,
      );
      expect(sx, closeTo(200, 0.001));
      expect(sy, closeTo(200, 0.001));
    });
  });

  group('What a rotation drag means', () {
    const anchor = Offset(300, 200);

    test('it carries on past a full turn rather than wrapping', () {
      final result = rotationForDrag(
        anchor: anchor,
        from: const Offset(300, 100),
        to: const Offset(400, 200),
        current: 350,
        uniform: false,
      );
      expect(result, closeTo(440, 0.001),
          reason: 'a layer wound twice round keeps its winding');
    });
  });

  /// **A scale in flight, and a scale that flips.** The same rule the
  /// turn follows: the picture is previewed at the value being dragged towards,
  /// so the box has to be drawn there too. And a handle dragged *past* the
  /// anchor turns the layer over — which the map used to make impossible by
  /// flooring the factor just above zero, so a layer could be squashed to
  /// nothing and never mirrored.
  group('A box scaled to a size it has not been committed at', () {
    test('a negative scale turns the layer over rather than collapsing it', () {
      final flipped = box(size: const Size(200, 100)).scaledTo(-100, 100);
      // The layer's own top-left corner is now on the right of the anchor.
      expect(flipped.corners.first.dx, closeTo(400, 1e-6));
      expect(flipped.corners[1].dx, closeTo(200, 1e-6));
      expect(flipped.corners.first.dy, closeTo(150, 1e-6),
          reason: 'the axis that was not flipped is untouched');
    });

    test('zero is the one factor barred, because the map inverts it', () {
      expect(nonZeroScale(0), isNot(0));
      expect(nonZeroScale(-0.0), isNot(0));
      expect(nonZeroScale(-2), -2, reason: 'a real factor is left alone');
      final collapsed = box().scaledTo(0, 0);
      expect(collapsed.map.layerOf(const Offset(300, 200)).dx.isFinite, isTrue);
    });
  });
}
