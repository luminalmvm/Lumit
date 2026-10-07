// How the Viewer's picture texture is sampled.
//
// Below 100 % the picture is minified, and the nearest sampling the Viewer used
// everywhere kept one source pixel in every few and dropped the rest — not a
// smaller picture but a different one, which is what "soft and slightly odd"
// was. These pin the flag so it cannot silently go back to nearest, and pin the
// arithmetic that decides it: the zoom, the preview divisor and the device
// pixel ratio multiply, so a half-resolution frame at 80 % is magnified rather
// than minified and is filtered as such.

import 'package:flutter/painting.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_stage.dart';

void main() {
  FilterQuality filter({
    double shown = 1,
    int tier = 1,
    double dpr = 1,
    bool smooth = false,
  }) =>
      viewerPictureFilter(
        shownScale: shown,
        tier: tier,
        devicePixelRatio: dpr,
        smooth: smooth,
      );

  group('The picture below 100 %', () {
    test('is filtered rather than point-sampled', () {
      expect(filter(shown: 0.8), FilterQuality.medium);
      expect(filter(shown: 0.5), FilterQuality.medium);
      expect(filter(shown: 0.1), FilterQuality.medium);
    });
  });

  group('The three scales multiply', () {
    test('a half-resolution frame at 80 % is magnified, not minified', () {
      expect(filter(shown: 0.8, tier: 2), FilterQuality.none);
    });
  });
}
