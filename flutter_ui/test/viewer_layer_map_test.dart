// Unit tests for the ported egui `LayerMap` maths (viewer_layer_map.dart) —
// hand-computed cases for the layer↔screen round trip, the pan-behind position,
// and the scale-handle solve. No widget tree: pure geometry.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_layer_map.dart';

void expectOffset(Offset a, Offset b, {double eps = 1e-9}) {
  expect((a.dx - b.dx).abs() < eps, isTrue, reason: 'dx: ${a.dx} vs ${b.dx}');
  expect((a.dy - b.dy).abs() < eps, isTrue, reason: 'dy: ${a.dy} vs ${b.dy}');
}

void main() {
  group('ViewerLayerMap.toScreen / layerOf', () {
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
  });

  group('scaleForHandle', () {
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
}
