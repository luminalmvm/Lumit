// The painting tools' arithmetic (K-227): which mode each tool commits, and the
// thinning every stroke goes through before it crosses the bridge.
//
// A stroke is a record of a gesture, and a gesture arrives as hundreds of
// pointer events a second. What is stored has to be the *shape* of it — which is
// what thinning decides, and what would silently bloat every project file if it
// were wrong.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_paint.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/tools.dart';

void main() {
  group('Which mark each tool makes', () {
    test('the three painting tools commit the three modes', () {
      expect(paintModeFor(ToolMode.brush), BridgePaintMode.paint);
      expect(paintModeFor(ToolMode.eraser), BridgePaintMode.erase);
      expect(paintModeFor(ToolMode.cloneStamp), BridgePaintMode.clone);
    });
  });

  group('Thinning a stroke', () {
    test('drops the samples too close together to show', () {
      final thinned = thinStroke(const [
        Offset(0, 0),
        Offset(0.5, 0),
        Offset(1, 0),
        Offset(10, 0),
        Offset(10.5, 0),
        Offset(20, 0),
      ]);
      expect(thinned, const [
        Offset(0, 0),
        Offset(10, 0),
        Offset(20, 0),
      ]);
    });

    test('always keeps where the stroke started and where it stopped', () {
      // The last point is within the minimum of the one kept before it, and
      // must survive anyway: a stroke that stops short of the pointer is a
      // stroke that does not go where it was drawn.
      final thinned = thinStroke(const [
        Offset(0, 0),
        Offset(10, 0),
        Offset(10.5, 0),
      ]);
      expect(thinned.first, const Offset(0, 0));
      expect(thinned.last, const Offset(10.5, 0));
    });

    test('a single dab is left exactly as it is', () {
      expect(thinStroke(const [Offset(3, 4)]), const [Offset(3, 4)]);
      expect(thinStroke(const []), isEmpty);
    });

    test('a long slow drag thins to a fraction of its samples', () {
      // Five hundred events across a hundred pixels: the shape survives, the
      // bulk does not.
      final dense = [
        for (var i = 0; i < 500; i++) Offset(i * 0.2, 0),
      ];
      final thinned = thinStroke(dense);
      expect(thinned.length, lessThan(dense.length / 5));
      expect(thinned.last, dense.last);
    });
  });
}
