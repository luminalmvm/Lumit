// Unit tests for the ported egui `LayerMap` maths (viewer_layer_map.dart) —
// hand-computed cases for the layer↔screen round trip, the pan-behind position,
// and the scale-handle solve. No widget tree: pure geometry.

import 'dart:math' as math;

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_layer_map.dart';

void expectOffset(Offset a, Offset b, {double eps = 1e-9}) {
  expect((a.dx - b.dx).abs() < eps, isTrue, reason: 'dx: ${a.dx} vs ${b.dx}');
  expect((a.dy - b.dy).abs() < eps, isTrue, reason: 'dy: ${a.dy} vs ${b.dy}');
}

void main() {
  group('ViewerLayerMap.toScreen / layerOf', () {
    test('identity translate: position offsets, no scale/rotation', () {
      final m = ViewerLayerMap.of(
        positionX: 100,
        positionY: 50,
        anchorX: 0,
        anchorY: 0,
        scaleXPercent: 100,
        scaleYPercent: 100,
        rotationDegrees: 0,
        origin: Offset.zero,
        viewScale: 1,
      );
      // The anchor (0,0) lands at the position; a layer point adds on top.
      expectOffset(m.toScreen(0, 0), const Offset(100, 50));
      expectOffset(m.toScreen(10, 20), const Offset(110, 70));
      // Round trip.
      expectOffset(m.layerOf(const Offset(110, 70)), const Offset(10, 20));
    });

    test('scale doubles the layer-space offset', () {
      final m = ViewerLayerMap.of(
        positionX: 0,
        positionY: 0,
        anchorX: 0,
        anchorY: 0,
        scaleXPercent: 200,
        scaleYPercent: 200,
        rotationDegrees: 0,
        origin: Offset.zero,
        viewScale: 1,
      );
      expectOffset(m.toScreen(10, 5), const Offset(20, 10));
      expectOffset(m.layerOf(const Offset(20, 10)), const Offset(10, 5));
    });

    test('rotation by 90 degrees rotates the offset', () {
      final m = ViewerLayerMap.of(
        positionX: 0,
        positionY: 0,
        anchorX: 0,
        anchorY: 0,
        scaleXPercent: 100,
        scaleYPercent: 100,
        rotationDegrees: 90,
        origin: Offset.zero,
        viewScale: 1,
      );
      // (10, 0) rotates to (0, 10): rx = 10·cos - 0·sin = 0, ry = 10·sin = 10.
      expectOffset(m.toScreen(10, 0), const Offset(0, 10), eps: 1e-9);
      expectOffset(m.layerOf(const Offset(0, 10)), const Offset(10, 0),
          eps: 1e-9);
    });

    test('origin and viewScale place and scale into screen space', () {
      final m = ViewerLayerMap.of(
        positionX: 10,
        positionY: 10,
        anchorX: 0,
        anchorY: 0,
        scaleXPercent: 100,
        scaleYPercent: 100,
        rotationDegrees: 0,
        origin: const Offset(5, 5),
        viewScale: 2,
      );
      // origin + (position + 0) * viewScale = (5,5) + (10,10)*2 = (25,25).
      expectOffset(m.toScreen(0, 0), const Offset(25, 25));
      expectOffset(m.layerOf(const Offset(25, 25)), const Offset(0, 0));
    });

    test('full round trip under scale + rotation + anchor + view', () {
      final m = ViewerLayerMap.of(
        positionX: 300,
        positionY: 220,
        anchorX: 40,
        anchorY: 30,
        scaleXPercent: 150,
        scaleYPercent: 80,
        rotationDegrees: 33,
        origin: const Offset(12, 7),
        viewScale: 1.7,
      );
      for (final p in const [
        Offset(0, 0),
        Offset(120, 60),
        Offset(-30, 200),
      ]) {
        final back = m.layerOf(m.toScreen(p.dx, p.dy));
        expectOffset(back, p, eps: 1e-6);
      }
    });
  });

  group('panBehindPosition', () {
    test('no rotation: the scaled anchor delta adds straight to position', () {
      final p = panBehindPosition(
        oldAnchor: const Offset(0, 0),
        newAnchor: const Offset(10, 0),
        position: const Offset(0, 0),
        scaleXPercent: 100,
        scaleYPercent: 100,
        rotationDegrees: 0,
      );
      expectOffset(p, const Offset(10, 0));
    });

    test('90 degrees rotates the anchor delta before adding', () {
      final p = panBehindPosition(
        oldAnchor: const Offset(0, 0),
        newAnchor: const Offset(10, 0),
        position: const Offset(0, 0),
        scaleXPercent: 100,
        scaleYPercent: 100,
        rotationDegrees: 90,
      );
      expectOffset(p, const Offset(0, 10), eps: 1e-9);
    });

    test('scale halves the anchor delta', () {
      final p = panBehindPosition(
        oldAnchor: const Offset(0, 0),
        newAnchor: const Offset(20, 40),
        position: const Offset(5, 5),
        scaleXPercent: 50,
        scaleYPercent: 50,
        rotationDegrees: 0,
      );
      expectOffset(p, const Offset(15, 25));
    });
  });

  group('scaleForHandle', () {
    test('dragging a corner to double-x yields 200% on x, x-only', () {
      final m = ViewerLayerMap.of(
        positionX: 0,
        positionY: 0,
        anchorX: 0,
        anchorY: 0,
        scaleXPercent: 100,
        scaleYPercent: 100,
        rotationDegrees: 0,
        origin: Offset.zero,
        viewScale: 1,
      );
      // The corner sits at layer (100, 50); its screen point is (100, 50).
      // Drag it to (200, 50): x doubles, y stays.
      final (sx, sy) = m.scaleForHandle(
        dxFromAnchor: 100,
        dyFromAnchor: 50,
        pointer: const Offset(200, 50),
      );
      expect(sx, closeTo(200, 1e-6));
      expect(sy, closeTo(100, 1e-6));
    });

    test('a zero perpendicular offset keeps that axis fixed (edge handle)', () {
      final m = ViewerLayerMap.of(
        positionX: 0,
        positionY: 0,
        anchorX: 0,
        anchorY: 0,
        scaleXPercent: 100,
        scaleYPercent: 100,
        rotationDegrees: 0,
        origin: Offset.zero,
        viewScale: 1,
      );
      // A right-edge handle scales only x (dy passed as 0): y must not move.
      final (sx, sy) = m.scaleForHandle(
        dxFromAnchor: 100,
        dyFromAnchor: 0,
        pointer: const Offset(150, 999),
      );
      expect(sx, closeTo(150, 1e-6));
      expect(sy, closeTo(100, 1e-6));
    });

    test('scale-handle solve is consistent with toScreen under rotation', () {
      final m = ViewerLayerMap.of(
        positionX: 200,
        positionY: 100,
        anchorX: 25,
        anchorY: 15,
        scaleXPercent: 100,
        scaleYPercent: 100,
        rotationDegrees: 40,
        origin: const Offset(3, 9),
        viewScale: 1.3,
      );
      // Choose a target scale, compute where the corner would land, then check
      // the solve recovers that scale from that pointer.
      const cornerX = 90.0, cornerY = 70.0;
      const wantSx = 175.0, wantSy = 60.0;
      final scaled = ViewerLayerMap.of(
        positionX: 200,
        positionY: 100,
        anchorX: 25,
        anchorY: 15,
        scaleXPercent: wantSx,
        scaleYPercent: wantSy,
        rotationDegrees: 40,
        origin: const Offset(3, 9),
        viewScale: 1.3,
      );
      final pointer = scaled.toScreen(cornerX, cornerY);
      final (sx, sy) = m.scaleForHandle(
        dxFromAnchor: cornerX - 25,
        dyFromAnchor: cornerY - 15,
        pointer: pointer,
      );
      expect(sx, closeTo(wantSx, 1e-6));
      expect(sy, closeTo(wantSy, 1e-6));
    });
  });

  test('rotation sanity: 180 degrees flips the sign of an offset', () {
    final m = ViewerLayerMap.of(
      positionX: 0,
      positionY: 0,
      anchorX: 0,
      anchorY: 0,
      scaleXPercent: 100,
      scaleYPercent: 100,
      rotationDegrees: 180,
      origin: Offset.zero,
      viewScale: 1,
    );
    final s = m.toScreen(10, 4);
    expectOffset(s, const Offset(-10, -4), eps: 1e-6);
    // cos(180) = -1, sin(180) ~ 0.
    expect(m.cos, closeTo(-1, 1e-9));
    expect(m.sin.abs() < 1e-9, isTrue);
    expect(math.pi, closeTo(3.14159265, 1e-6));
  });
}
