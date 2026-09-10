// The camera tools' arithmetic in the eye model: the camera's own axes, where
// each drag puts the eye, and which drag a mouse button asks for.
//
// The axes are the part that has to agree with the *renderer* — lumit-gpu builds
// the camera matrix as `Ry · Rx · Rz`, and a tool that moved the camera along a
// different set of axes would send it sideways when you asked for forward. So
// they are checked against hand-computed cases here rather than by dragging.
//
// The rest is the eye model itself (docs/impl/camera.md §4): the layer's
// position is the eye, the pivot is what it is aimed at, and an orbit is the
// one drag that has to leave the pivot exactly where it was.

import 'dart:math' as math;

import 'package:flutter/gestures.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_camera.dart';
import 'package:lumit_flutter/state/tools.dart';

void main() {
  // A camera at the origin of an HD comp, looking down +z at the plane 2667
  // ahead of it - which is where a fresh camera sits.
  CameraPose pose({
    (double, double, double) position = (960, 540, -2667),
    (double, double, double) rotation = (0, 0, 0),
    double zoom = 2667,
    (double, double, double) pointOfInterest = (960, 540, 0),
    bool twoNode = false,
  }) =>
      CameraPose(
        position: position,
        rotation: rotation,
        zoom: zoom,
        pointOfInterest: pointOfInterest,
        twoNode: twoNode,
      );

  void closeTriple(
    (double, double, double) got,
    (double, double, double) want, {
    double tolerance = 1e-9,
  }) {
    expect(got.$1, closeTo(want.$1, tolerance));
    expect(got.$2, closeTo(want.$2, tolerance));
    expect(got.$3, closeTo(want.$3, tolerance));
  }

  double distance((double, double, double) a, (double, double, double) b) =>
      math.sqrt(math.pow(a.$1 - b.$1, 2) +
          math.pow(a.$2 - b.$2, 2) +
          math.pow(a.$3 - b.$3, 2));

  group('The camera\'s own axes', () {
    test('an unrotated camera looks down +z, with x right and y down', () {
      final axes = pose().axes;
      closeTriple(axes.right, (1, 0, 0));
      closeTriple(axes.up, (0, 1, 0));
      closeTriple(axes.forward, (0, 0, 1));
    });

    test('a quarter turn about y points it along +x', () {
      final axes = pose(rotation: (0, 90, 0)).axes;
      closeTriple(axes.forward, (1, 0, 0));
      closeTriple(axes.right, (0, 0, -1));
    });

    test('a quarter turn about x points it along -y', () {
      final axes = pose(rotation: (90, 0, 0)).axes;
      closeTriple(axes.forward, (0, -1, 0));
      closeTriple(axes.right, (1, 0, 0));
    });

    test('the three axes stay perpendicular under any rotation', () {
      final axes = pose(rotation: (20, -35, 12)).axes;
      double dot((double, double, double) a, (double, double, double) b) =>
          a.$1 * b.$1 + a.$2 * b.$2 + a.$3 * b.$3;
      expect(dot(axes.right, axes.up), closeTo(0, 1e-9));
      expect(dot(axes.right, axes.forward), closeTo(0, 1e-9));
      expect(dot(axes.up, axes.forward), closeTo(0, 1e-9));
      expect(math.sqrt(dot(axes.forward, axes.forward)), closeTo(1, 1e-9));
    });
  });

  group('The pivot', () {
    test('a one-node camera turns round the plane it renders 1:1', () {
      closeTriple(pose().pivot, (960, 540, 0));
      // Turned to look along +x, the plane is off to its right.
      closeTriple(
        pose(position: (0, 540, 0), rotation: (0, 90, 0)).pivot,
        (2667, 540, 0),
      );
    });

    test('a two-node camera turns round what it is aimed at', () {
      final aimed = pose(twoNode: true, pointOfInterest: (100, 200, 300));
      closeTriple(aimed.pivot, (100, 200, 300));
    });
  });

  group('Orbit', () {
    test('swings the eye round the pivot and leaves the pivot alone', () {
      final start = pose();
      final turned = orbitCamera(start, 100, 0);
      closeTriple(turned.pivot, start.pivot, tolerance: 1e-9);
      expect(turned.rotation.$2, closeTo(100 * orbitDegreesPerPixel, 1e-9),
          reason: 'a one-node camera takes the new angles');
      expect(turned.position, isNot(start.position),
          reason: 'and the eye moves to keep the pivot in front');
      expect(distance(turned.position, turned.pivot), closeTo(start.zoom, 1e-9),
          reason: 'still the focal distance away');
    });

    test('dragging up lifts the eye over the top', () {
      final up = orbitCamera(pose(), 0, -100);
      expect(up.rotation.$1, lessThan(0),
          reason: 'over the top means tilted to look down');
      expect(up.position.$2, lessThan(pose().position.$2),
          reason: 'the eye is higher up the screen, which is lower y');
      // And the other way round, so nobody ships an inverted orbit.
      final down = orbitCamera(pose(), 0, 100);
      expect(down.position.$2, greaterThan(pose().position.$2));
    });

    test('the pitch stops short of straight down', () {
      final over = orbitCamera(pose(rotation: (-80, 0, 0)), 0, -10000);
      expect(over.rotation.$1, -89.9,
          reason: 'past the pole the picture would flip over');
      final under = orbitCamera(pose(rotation: (80, 0, 0)), 0, 10000);
      expect(under.rotation.$1, 89.9);
    });

    test('Shift keeps the sweep on one axis', () {
      final level = orbitCamera(pose(), 100, 8, lockAxis: true);
      expect(level.rotation.$1, 0, reason: 'the smaller movement is dropped');
      expect(level.rotation.$2, closeTo(100 * orbitDegreesPerPixel, 1e-9));
    });

    test('a two-node camera moves its eye and keeps its rotation rows', () {
      // Aimed at the middle of the comp from 1000 back, with rows of its own on
      // top of the aim.
      final start = pose(
        position: (960, 540, -1000),
        rotation: (5, -10, 0),
        twoNode: true,
        pointOfInterest: (960, 540, 0),
      );
      final turned = orbitCamera(start, 100, 40);
      expect(turned.rotation, start.rotation,
          reason: 'the aim follows the eye, so the rows are left alone');
      expect(turned.pointOfInterest, start.pointOfInterest);
      expect(
        distance(turned.position, start.pointOfInterest),
        closeTo(distance(start.position, start.pointOfInterest), 1e-9),
        reason: 'it circles the point at the distance it already had',
      );
      expect(turned.position, isNot(start.position));
    });
  });

  group('Track', () {
    test('slides the eye across its own axes, against the drag', () {
      // At 1:1, dragging 50px right moves the camera 50px left, so the picture
      // moves with the pointer.
      final moved = trackCamera(pose(), 50, 20, scale: 1);
      closeTriple(moved.position, (960 - 50, 540 - 20, -2667));
      expect(moved.rotation, pose().rotation, reason: 'a track never turns');
    });

    test('the magnification is undone, so the picture keeps up', () {
      // Zoomed to half size, 50 screen pixels is 100 comp pixels.
      final moved = trackCamera(pose(), 50, 0, scale: 0.5);
      expect(moved.position.$1, closeTo(960 - 100, 1e-9));
    });

    test('a turned camera tracks along its own axes, not the comp\'s', () {
      // Looking along +x: dragging right slides the camera along z.
      final moved = trackCamera(pose(rotation: (0, 90, 0)), 50, 0, scale: 1);
      expect(moved.position.$1, closeTo(960, 1e-9));
      expect(moved.position.$3, closeTo(-2667 + 50, 1e-9));
    });

    test('Shift keeps it on one axis', () {
      final moved = trackCamera(pose(), 50, 9, scale: 1, lockAxis: true);
      expect(moved.position.$2, 540);
    });

    test('a two-node camera carries what it is aimed at with it', () {
      final start = pose(twoNode: true);
      final moved = trackCamera(start, 50, 20, scale: 1);
      closeTriple(moved.pointOfInterest, (960 - 50, 540 - 20, 0),
          tolerance: 1e-9);
      expect(distance(moved.position, moved.pointOfInterest),
          closeTo(distance(start.position, start.pointOfInterest), 1e-9),
          reason: 'the framing moves rather than the aim');
    });

    test('a one-node camera leaves its stored point of interest alone', () {
      final moved = trackCamera(pose(), 50, 20, scale: 1);
      expect(moved.pointOfInterest, pose().pointOfInterest);
    });
  });

  group('Dolly', () {
    test('moves the eye along the view axis, in proportion to the distance',
        () {
      final inward = dollyCamera(pose(), 100, 0);
      expect(inward.position.$3,
          closeTo(-2667 + 100 * dollyFraction * 2667, 1e-9),
          reason: 'dragging right goes in, along +z for an unturned camera');
      expect(inward.rotation, pose().rotation);
    });

    test('a nearer camera creeps and a further one covers ground', () {
      final near = dollyCamera(pose(position: (960, 540, 0), zoom: 100), 100, 0)
          .position
          .$3;
      final far =
          dollyCamera(pose(position: (960, 540, 0), zoom: 10000), 100, 0)
              .position
              .$3;
      expect(far, greaterThan(near * 10));
    });

    test('whichever axis carries the drag is the one that counts', () {
      final across = dollyCamera(pose(), 80, 5);
      final down = dollyCamera(pose(), 5, 80);
      expect(across.position.$3, closeTo(down.position.$3, 1e-9));
    });

    test('a turned camera dollies along where it is pointed', () {
      final moved = dollyCamera(pose(rotation: (0, 90, 0)), 100, 0);
      expect(moved.position.$1, greaterThan(960));
      expect(moved.position.$3, closeTo(-2667, 1e-9));
    });

    test('what a two-node camera is aimed at stays where it is', () {
      final start = pose(twoNode: true);
      final moved = dollyCamera(start, 100, 0);
      expect(moved.pointOfInterest, start.pointOfInterest);
      expect(distance(moved.position, moved.pointOfInterest),
          lessThan(distance(start.position, start.pointOfInterest)),
          reason: 'a dolly is how the distance to the point is changed');
    });
  });

  group('The unified tool', () {
    test('picks its move by the button that started the drag', () {
      expect(cameraMoveForButtons(kPrimaryMouseButton), ToolMode.cameraOrbit);
      expect(cameraMoveForButtons(kMiddleMouseButton), ToolMode.cameraPan);
      expect(cameraMoveForButtons(kSecondaryMouseButton), ToolMode.cameraDolly);
    });

    test('leads the camera group, so its chord and its button start there', () {
      expect(ToolMode.builtMembersOf(ToolGroup.camera).first,
          ToolMode.cameraUnified);
      expect(ToolsState().memberOf(ToolGroup.camera), ToolMode.cameraUnified);
    });
  });
}
