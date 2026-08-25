// The Viewer gizmo's arithmetic (K-217): which layer a point is inside, what a
// marquee catches, where the handles sit once a layer is turned, and what a
// handle drag means.
//
// All of it is pure, so all of it is checked here against hand-computed cases
// rather than by dragging in a widget tree — the same reasoning as
// viewer_layer_map.dart, whose maths these build on.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/panels/viewer_gizmo.dart';
import 'package:lumit_flutter/panels/viewer_layer_map.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

void main() {
  /// A layer of [size] sitting at [at] in a comp drawn 1:1 from the origin,
  /// anchored on its own middle — the arrangement a placed clip gets (K-150).
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
      );

  group('What a point is inside', () {
    test('the layer contains its own middle and not the space beside it', () {
      final b = box();
      expect(b.contains(const Offset(300, 200)), isTrue);
      expect(b.contains(const Offset(399, 249)), isTrue,
          reason: 'just inside the bottom-right corner');
      expect(b.contains(const Offset(401, 200)), isFalse);
      expect(b.contains(const Offset(300, 260)), isFalse);
    });

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

    test('a rotated box is caught by its corners, not its axis-aligned span',
        () {
      final b = box(rotation: 45);
      // The turned box reaches about ±106 px from its middle on the diagonal.
      expect(b.insideRect(const Rect.fromLTRB(180, 80, 420, 320)), isTrue);
      expect(b.insideRect(const Rect.fromLTRB(220, 140, 380, 260)), isFalse);
    });

    test('layersInsideRect keeps stacking order', () {
      final top = box(size: const Size(20, 20));
      final under = box(size: const Size(40, 40));
      final caught =
          layersInsideRect([top, under], const Rect.fromLTRB(0, 0, 600, 400));
      expect(caught.map((b) => b.id).toList(), [top.id, under.id]);
    });
  });

  /// A mask's own points (K-224): with the Selection tool and the wireframes
  /// on, every vertex of every mask is a thing you can aim at, sweep up and
  /// drag. The arithmetic that decides *which* is here.
  group('A mask\'s points', () {
    test('sit where the layer\'s map puts them, not where the path says', () {
      // The layer is 200x100 at (300, 200), so its own origin is at (200, 150)
      // on screen and a vertex at (20, 20) lands at (220, 170).
      final b = box(masks: [squareMask()]);
      final points = pathPointsOf(b);
      expect(points.length, 4);
      expect(points.first.at, const Offset(220, 170));
      expect(points.first.index, 0);
      expect(points[2].at, const Offset(280, 230));
    });

    test('travel with the layer\'s rotation', () {
      final b = box(rotation: 90, masks: [squareMask()]);
      // The vertex sits 80 left and 30 above the anchor; a quarter turn puts
      // it 30 right and 80 above instead.
      final at = pathPointsOf(b).first.at;
      expect(at.dx, closeTo(330, 1e-9));
      expect(at.dy, closeTo(120, 1e-9));
    });

    test('a press near one names it, and one far away names nothing', () {
      final b = box(masks: [squareMask()]);
      final hit = pathPointAt([b], const Offset(223, 172));
      expect(hit, isNotNull);
      expect(hit!.index, 0);
      expect(hit.key, maskPointKey(b.id, b.masks.single.id, 0));
      expect(pathPointAt([b], const Offset(250, 200)), isNull,
          reason: 'the middle of the mask is not one of its points');
    });

    test('the nearest wins when two are close together', () {
      final b = box(masks: [squareMask(side: 8)]);
      // The four vertices are 8 px apart; a press to the right of the second
      // one must name the second, not the first.
      expect(pathPointAt([b], const Offset(229, 170))!.index, 1);
      expect(pathPointAt([b], const Offset(221, 170))!.index, 0);
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

    test('a layer with no mask has no points to catch', () {
      expect(pathPointsOf(box()), isEmpty);
      expect(pathPointAt([box()], const Offset(300, 200)), isNull);
    });
  });

  /// A shape layer's own art is editable on the picture by the same gesture
  /// (K-237's "the same gesture over shape contents is the next piece"). The
  /// arithmetic is shared with masks; what these pin is that a shape item's
  /// points are found, named apart from a mask's, and swept up the same way.
  group("A shape layer's points", () {
    test('sit where a mask\'s would, because the path type is the same', () {
      final b = box(shapeContents: [squareShape()]);
      final points = pathPointsOf(b);
      expect(points.length, 4);
      expect(points.first.at, const Offset(220, 170));
      expect(points.first.shape, isTrue,
          reason: 'the point knows which list it came from, because that is '
              'what decides where the edit is written back');
      expect(points[2].at, const Offset(280, 230));
    });

    test('a press near one names it', () {
      final b = box(shapeContents: [squareShape()]);
      final hit = pathPointAt([b], const Offset(223, 172));
      expect(hit, isNotNull);
      expect(hit!.index, 0);
      expect(hit.shape, isTrue);
      expect(hit.key, shapePointKey(b.id, b.shapeContents.single.id, 0));
    });

    test('a shape point and a mask point are never the same point', () {
      final id = UuidValue.fromString(const Uuid().v4());
      final layer = UuidValue.fromString(const Uuid().v4());
      // Same layer, same path id, same index — and still two different points,
      // because one is written back with setMask and the other with
      // setShapeContents. Without the prefix a selection could not tell them
      // apart and one would be committed as the other.
      expect(maskPointKey(layer, id, 0), isNot(shapePointKey(layer, id, 0)));
    });

    test('a layer carrying both offers the points of both', () {
      final b = box(masks: [squareMask()], shapeContents: [squareShape()]);
      final points = pathPointsOf(b);
      expect(points.length, 8);
      expect(points.where((p) => p.shape).length, 4);
      expect(points.where((p) => !p.shape).length, 4);
    });

    test('a sweep gathers a shape\'s points as readily as a mask\'s', () {
      final b = box(shapeContents: [squareShape()]);
      final caught =
          pathPointsInRect([b], const Rect.fromLTRB(200, 150, 300, 200));
      expect(caught, {
        shapePointKey(b.id, b.shapeContents.single.id, 0),
        shapePointKey(b.id, b.shapeContents.single.id, 1),
      });
    });

    test('a layer with no art has no points to catch', () {
      expect(pathPointsOf(box()), isEmpty);
    });

    /// **The art's coordinates are not the layer's pixels** (K-308). The engine
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
    test('the eight scale handles land on the box corners and edge middles',
        () {
      final b = box();
      expect(b.handleAt(GizmoHandle.topLeft), const Offset(200, 150));
      expect(b.handleAt(GizmoHandle.top), const Offset(300, 150));
      expect(b.handleAt(GizmoHandle.bottomRight), const Offset(400, 250));
      expect(b.handleAt(GizmoHandle.left), const Offset(200, 200));
    });

    test('the rotation knob stands off the top edge, and turns with the layer',
        () {
      final upright = box().handleAt(GizmoHandle.rotate);
      expect(upright.dx, closeTo(300, 0.001));
      expect(upright.dy, closeTo(150 - gizmoRotateReach, 0.001),
          reason: 'straight up from the top edge while the layer is upright');

      // Turned a half-circle, "up" for the layer is down the screen.
      final flipped = box(rotation: 180).handleAt(GizmoHandle.rotate);
      expect(flipped.dy, closeTo(250 + gizmoRotateReach, 0.001));
    });

    test('a press near a handle finds it, and one far from any finds none', () {
      final b = box();
      expect(b.handleHit(const Offset(202, 152)), GizmoHandle.topLeft);
      expect(b.handleHit(const Offset(290, 180)), isNull,
          reason: 'open ground inside the layer is not a handle');
    });

    /// The anchor became a handle with K-221, and it sits where a body drag
    /// begins — so it has to be *aimed at* rather than fallen into, or every
    /// drag of a layer would pan behind instead of moving it.
    test('the anchor is a handle, but only within a tight radius', () {
      final b = box();
      expect(b.handleHit(const Offset(300, 200)), GizmoHandle.anchor,
          reason: 'dead on the pivot');
      expect(b.handleHit(const Offset(304, 202)), GizmoHandle.anchor);
      expect(b.handleHit(const Offset(316, 200)), isNull,
          reason: 'a shade further out is a move, not a pan-behind');
    });

    test('the anchor handle follows the anchor, not the middle of the box', () {
      // A layer whose pivot is its top-left corner.
      final b = LayerBox(
        layer: LayerReference(
          internalprojectId: UuidValue.fromString(const Uuid().v4()),
          internalcompId: UuidValue.fromString(const Uuid().v4()),
          internallayerId: UuidValue.fromString(const Uuid().v4()),
        ),
        id: UuidValue.fromString(const Uuid().v4()),
        map: ViewerLayerMap.of(
          positionX: 300,
          positionY: 200,
          anchorX: 0,
          anchorY: 0,
          scaleXPercent: 100,
          scaleYPercent: 100,
          rotationDegrees: 0,
          origin: Offset.zero,
          viewScale: 1,
        ),
        bounds: const Size(200, 100),
        draggable: true,
        scalable: true,
        rotationDegrees: 0,
      );
      expect(b.handleAt(GizmoHandle.anchor), const Offset(300, 200));
      expect(b.handleAt(GizmoHandle.topLeft), const Offset(300, 200),
          reason: 'which is also the corner, here');
      expect(b.handleAt(GizmoHandle.bottomRight), const Offset(500, 300));
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

    test('an edge handle moves only its own axis', () {
      final b = box();
      final (sx, sy) = scaleForGizmoHandle(
        box: b,
        handle: GizmoHandle.right,
        pointer: const Offset(500, 400),
        uniform: false,
      );
      expect(sx, closeTo(200, 0.001));
      expect(sy, closeTo(100, 0.001),
          reason: 'the vertical has no offset from the anchor to resolve');
    });

    test('Shift keeps the proportions', () {
      final b = box();
      // A corner dragged to an off-diagonal point asks for 200% across and
      // 100% down; held uniform, both take the mean.
      final (sx, sy) = scaleForGizmoHandle(
        box: b,
        handle: GizmoHandle.bottomRight,
        pointer: const Offset(500, 250),
        uniform: true,
      );
      expect(sx, closeTo(sy, 0.001), reason: 'that is what uniform means');
      expect(sx, closeTo(150, 0.001));
    });

    test('Shift on an edge handle drives both axes from the resolved one', () {
      final b = box();
      final (sx, sy) = scaleForGizmoHandle(
        box: b,
        handle: GizmoHandle.right,
        pointer: const Offset(500, 200),
        uniform: true,
      );
      expect(sx, closeTo(200, 0.001));
      expect(sy, closeTo(200, 0.001),
          reason: 'the unresolved axis follows rather than staying behind');
    });
  });

  group('What a rotation drag means', () {
    const anchor = Offset(300, 200);

    test('the angle swept is added to where the layer already was', () {
      final result = rotationForDrag(
        anchor: anchor,
        from: const Offset(300, 100), // straight up
        to: const Offset(400, 200), // to the right: a quarter turn clockwise
        current: 0,
        uniform: false,
      );
      expect(result, closeTo(90, 0.001));
    });

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

    test('Shift snaps to 45° steps', () {
      final result = rotationForDrag(
        anchor: anchor,
        from: const Offset(300, 100),
        to: Offset(300 + 100 * math.cos(-0.6), 200 + 100 * math.sin(-0.6)),
        current: 0,
        uniform: true,
      );
      expect(result % 45, closeTo(0, 0.001));
    });
  });

  /// **A turn in flight (K-230).** The picture is previewed at the new angle
  /// while the drag is happening, so the box over it has to be drawn at that
  /// angle too — the document still holds the old one, and drawing from the
  /// document is what made the wireframe sit still until the button came up.
  group('A box turned to an angle it has not been committed at', () {
    test('its corners are where the turned layer would put them', () {
      final upright = box(size: const Size(200, 100));
      final turned = upright.turnedTo(90);
      // The layer is anchored on its own middle at (300, 200), so a quarter
      // turn takes the top-left corner from (200, 150) to (350, 100).
      expect(turned.corners.first.dx, closeTo(350, 1e-6));
      expect(turned.corners.first.dy, closeTo(100, 1e-6));
      expect(turned.rotationDegrees, 90);
    });

    test('it is the same box in every other respect', () {
      final upright = box();
      final turned = upright.turnedTo(37);
      expect(turned.id, upright.id);
      expect(turned.bounds, upright.bounds);
      expect(turned.anchorScreen, upright.anchorScreen,
          reason: 'a layer turns about its anchor, so the anchor does not move');
    });

    test('turning it to where it already is changes nothing', () {
      final at30 = box(rotation: 30);
      final again = at30.turnedTo(30);
      for (var i = 0; i < 4; i++) {
        expect(again.corners[i].dx, closeTo(at30.corners[i].dx, 1e-9));
        expect(again.corners[i].dy, closeTo(at30.corners[i].dy, 1e-9));
      }
    });
  });

  /// **A scale in flight (K-230), and a scale that flips.** The same rule the
  /// turn follows: the picture is previewed at the value being dragged towards,
  /// so the box has to be drawn there too. And a handle dragged *past* the
  /// anchor turns the layer over — which the map used to make impossible by
  /// flooring the factor just above zero, so a layer could be squashed to
  /// nothing and never mirrored.
  group('A box scaled to a size it has not been committed at', () {
    test('its corners are where the scaled layer would put them', () {
      final full = box(size: const Size(200, 100));
      final half = full.scaledTo(50, 50);
      // Anchored on its own middle at (300, 200): at half size the box runs
      // from (250, 175) to (350, 225).
      expect(half.corners.first.dx, closeTo(250, 1e-6));
      expect(half.corners.first.dy, closeTo(175, 1e-6));
      expect(half.corners[2].dx, closeTo(350, 1e-6));
      expect(half.corners[2].dy, closeTo(225, 1e-6));
    });

    test('a negative scale turns the layer over rather than collapsing it', () {
      final flipped = box(size: const Size(200, 100)).scaledTo(-100, 100);
      // The layer's own top-left corner is now on the right of the anchor.
      expect(flipped.corners.first.dx, closeTo(400, 1e-6));
      expect(flipped.corners[1].dx, closeTo(200, 1e-6));
      expect(flipped.corners.first.dy, closeTo(150, 1e-6),
          reason: 'the axis that was not flipped is untouched');
    });

    test('a mirrored layer still hit-tests where it is drawn', () {
      final flipped = box(size: const Size(200, 100)).scaledTo(-100, 100);
      expect(flipped.contains(const Offset(350, 200)), isTrue);
      expect(flipped.contains(const Offset(450, 200)), isFalse);
    });

    test('zero is the one factor barred, because the map inverts it', () {
      expect(nonZeroScale(0), isNot(0));
      expect(nonZeroScale(-0.0), isNot(0));
      expect(nonZeroScale(-2), -2, reason: 'a real factor is left alone');
      final collapsed = box().scaledTo(0, 0);
      expect(collapsed.map.layerOf(const Offset(300, 200)).dx.isFinite, isTrue);
    });
  });

  /// The in-flight copies used to be rebuilt field by field, and silently
  /// dropped `shapeContents` and `artOrigin` — so a shape layer's art (and its
  /// editable points) vanished from the overlay the moment a scale, turn or
  /// pivot drag began, and came back on release.
  group('A gesture in flight keeps the art', () {
    test('scale, turn and pivot all carry the shape contents and art origin',
        () {
      final b = box(
        size: const Size(60, 60),
        masks: [squareMask()],
        shapeContents: [squareShape(left: 120, top: 80, side: 60)],
        artOrigin: const Offset(120, 80),
      );
      for (final moved in [
        b.scaledTo(50, 50),
        b.turnedTo(90),
        b.pivotedAt(const Offset(10, 10), const Offset(290, 190)),
      ]) {
        expect(moved.shapeContents, b.shapeContents);
        expect(moved.artOrigin, b.artOrigin);
        expect(moved.masks, b.masks);
        // The user-visible half: the art's points are still there to aim at.
        expect(pathPointsOf(moved).where((p) => p.shape), hasLength(4));
      }
    });
  });
}
