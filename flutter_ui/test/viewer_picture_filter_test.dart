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

    test('is filtered whether or not the smoothing setting is on', () {
      expect(filter(shown: 0.8, smooth: true), FilterQuality.medium);
    });
  });

  group('The picture at or above 100 %', () {
    test('keeps its pixels square by default', () {
      expect(filter(), FilterQuality.none);
      expect(filter(shown: 8), FilterQuality.none);
    });

    test('smooths when the setting asks it to', () {
      expect(filter(smooth: true), FilterQuality.low);
      expect(filter(shown: 8, smooth: true), FilterQuality.low);
    });
  });

  group('The three scales multiply', () {
    test('a half-resolution frame at 80 % is magnified, not minified', () {
      expect(filter(shown: 0.8, tier: 2), FilterQuality.none);
    });

    test('a quarter-resolution frame at 20 % is still minified', () {
      expect(filter(shown: 0.2, tier: 4), FilterQuality.medium);
      expect(filter(shown: 0.3, tier: 4), FilterQuality.none);
    });

    test('a hi-dpi screen at 80 % is really 1.2 pixels a texel', () {
      expect(filter(shown: 0.8, dpr: 1.5), FilterQuality.none);
      expect(filter(shown: 0.7, dpr: 1.5), FilterQuality.none);
      expect(filter(shown: 0.6, dpr: 1.5), FilterQuality.medium);
    });

    test('an unknown or nonsense tier counts as full resolution', () {
      expect(filter(shown: 0.8, tier: 0), FilterQuality.medium);
      expect(filter(shown: 0.8, tier: -3), FilterQuality.medium);
    });
  });
}
