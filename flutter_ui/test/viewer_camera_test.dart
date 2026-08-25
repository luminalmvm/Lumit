// The camera tools' arithmetic (K-229): the camera's own axes, and what each of
// the three drags does to its pose.
//
// The axes are the part that has to agree with the *renderer* — lumit-gpu builds
// the camera matrix as `Ry · Rx · Rz`, and a tool that moved the camera along a
// different set of axes would send it sideways when you asked for forward. So
// they are checked against hand-computed cases here rather than by dragging.

import 'dart:math' as math;

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_camera.dart';

void main() {
  CameraPose pose({
    (double, double, double) position = (960, 540, 0),
    (double, double, double) rotation = (0, 0, 0),
    double distance = 2667,
  }) =>
      CameraPose(position: position, rotation: rotation, distance: distance);

  /// Where the eye is: the focal distance back along the camera's forward
  /// axis. Derived here rather than on [CameraPose] — the tools never need
  /// it, only these geometry checks do.
  (double, double, double) eyeOf(CameraPose p) {
    final f = p.axes.forward;
    return (
      p.position.$1 - f.$1 * p.distance,
      p.position.$2 - f.$2 * p.distance,
      p.position.$3 - f.$3 * p.distance,
    );
  }

  void closeTriple(
    (double, double, double) got,
    (double, double, double) want, {
    double tolerance = 1e-9,
  }) {
    expect(got.$1, closeTo(want.$1, tolerance));
    expect(got.$2, closeTo(want.$2, tolerance));
    expect(got.$3, closeTo(want.$3, tolerance));
  }

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

    test('the eye sits the focal distance behind what it looks at', () {
      // Unrotated, looking down +z from 2667 back.
      closeTriple(eyeOf(pose()), (960, 540, -2667));
      // Turned to look along +x, the eye moves round to the -x side.
      closeTriple(eyeOf(pose(rotation: (0, 90, 0))), (960 - 2667, 540, 0));
    });
  });

  group('Orbit', () {
    test('swings the camera round without moving what it looks at', () {
      final turned = orbitCamera(pose(), 100, 0);
      expect(turned.position, pose().position,
          reason: 'the pivot is the point being looked at, and it stays put');
      expect(turned.rotation.$2, closeTo(100 * orbitDegreesPerPixel, 1e-9));
      // The eye has swung: it is no longer straight behind.
      expect(eyeOf(turned).$1, isNot(closeTo(960, 1)));
    });

    test('dragging up lifts the camera over the top', () {
      final up = orbitCamera(pose(), 0, -100);
      expect(up.rotation.$1, lessThan(0),
          reason: 'over the top means tilted to look down');
      expect(eyeOf(up).$2, lessThan(eyeOf(pose()).$2),
          reason: 'the eye is higher up the screen, which is lower y');
      // And the other way round, so nobody ships an inverted orbit.
      final down = orbitCamera(pose(), 0, 100);
      expect(eyeOf(down).$2, greaterThan(eyeOf(pose()).$2));
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
  });

  group('Track', () {
    test('slides the camera across its own axes, against the drag', () {
      // At 1:1, dragging 50px right moves the camera 50px left, so the picture
      // moves with the pointer.
      final moved = trackCamera(pose(), 50, 20, scale: 1);
      closeTriple(moved.position, (960 - 50, 540 - 20, 0));
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
      expect(moved.position.$3, closeTo(50, 1e-9));
    });

    test('Shift keeps it on one axis', () {
      final moved = trackCamera(pose(), 50, 9, scale: 1, lockAxis: true);
      expect(moved.position.$2, 540);
    });
  });

  group('Dolly', () {
    test('moves along the view axis, in proportion to the distance', () {
      final inward = dollyCamera(pose(), 100, 0);
      expect(inward.position.$3,
          closeTo(100 * dollyFraction * 2667, 1e-9),
          reason: 'dragging right goes in, along +z for an unturned camera');
      expect(inward.rotation, pose().rotation);
    });

    test('a nearer camera creeps and a further one covers ground', () {
      final near = dollyCamera(pose(distance: 100), 100, 0).position.$3;
      final far = dollyCamera(pose(distance: 10000), 100, 0).position.$3;
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
      expect(moved.position.$3, closeTo(0, 1e-9));
    });
  });
}
