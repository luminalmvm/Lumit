// The painting tools' arithmetic: which mode each tool commits, and the
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
  });

  group('Reading the stylus', () {
    PointerDownEvent event(PointerDeviceKind kind,
            {double pressure = 1, double min = 0, double max = 1}) =>
        PointerDownEvent(
          kind: kind,
          pressure: pressure,
          pressureMin: min,
          pressureMax: max,
        );

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
