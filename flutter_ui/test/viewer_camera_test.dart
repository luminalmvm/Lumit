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
    test('a quarter turn about y points it along +x', () {
      final axes = pose(rotation: (0, 90, 0)).axes;
      closeTriple(axes.forward, (1, 0, 0));
      closeTriple(axes.right, (0, 0, -1));
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
  });

  group('Track', () {
    test('slides the eye across its own axes, against the drag', () {
      // At 1:1, dragging 50px right moves the camera 50px left, so the picture
      // moves with the pointer.
      final moved = trackCamera(pose(), 50, 20, scale: 1);
      closeTriple(moved.position, (960 - 50, 540 - 20, -2667));
      expect(moved.rotation, pose().rotation, reason: 'a track never turns');
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
  });

  group('The unified tool', () {
    test('picks its move by the button that started the drag', () {
      expect(cameraMoveForButtons(kPrimaryMouseButton), ToolMode.cameraOrbit);
      expect(cameraMoveForButtons(kMiddleMouseButton), ToolMode.cameraPan);
      expect(cameraMoveForButtons(kSecondaryMouseButton), ToolMode.cameraDolly);
    });
  });
}
