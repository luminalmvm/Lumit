// The painting tools' arithmetic (K-227): which mode each tool commits, and the
// thinning every stroke goes through before it crosses the bridge.
//
// A stroke is a record of a gesture, and a gesture arrives as hundreds of
// pointer events a second. What is stored has to be the *shape* of it — which is
// what thinning decides, and what would silently bloat every project file if it
// were wrong.

import 'package:flutter/gestures.dart';
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

    test('the indices are the same thinning, so a pressure rides along', () {
      final dense = [
        for (var i = 0; i < 200; i++) Offset(i * 0.7, i.isEven ? 0 : 0.3),
      ];
      final indices = thinStrokeIndices(dense);
      expect([for (final i in indices) dense[i]], thinStroke(dense));
      expect(indices.first, 0);
      expect(indices.last, dense.length - 1);
    });
  });

  group('Reading the stylus (K-583)', () {
    PointerDownEvent event(PointerDeviceKind kind,
            {double pressure = 1, double min = 0, double max = 1}) =>
        PointerDownEvent(
          kind: kind,
          pressure: pressure,
          pressureMin: min,
          pressureMax: max,
        );

    test('a mouse always presses fully, so nothing changes without a pen', () {
      expect(stylusPressure(event(PointerDeviceKind.mouse, pressure: 0.2)), 1);
      expect(stylusPressure(event(PointerDeviceKind.touch, pressure: 0)), 1);
    });

    test('a stylus reports where it is between its own two ends', () {
      expect(stylusPressure(event(PointerDeviceKind.stylus, pressure: 0.5)),
          closeTo(0.5, 1e-9));
      // A tablet whose range is not 0..1 is read against its own ends rather
      // than taken at face value.
      expect(
        stylusPressure(
            event(PointerDeviceKind.stylus, pressure: 512, min: 0, max: 1024)),
        closeTo(0.5, 1e-9),
      );
      // And one that reports no range at all is a full press, not a divide by
      // nothing.
      expect(
        stylusPressure(
            event(PointerDeviceKind.stylus, pressure: 7, min: 7, max: 7)),
        1,
      );
    });
  });
}
