// The Type tool's arithmetic (K-225): where a click falls in the composition,
// how wide a line is reckoned to be, and where that puts a new layer's anchor.
//
// All three are estimates the *engine* also makes — a text layer's anchor is
// placed by the same sum on the Rust side — so what these tests really pin is
// that the two sides agree. A caret that walks off the end of the line is what
// disagreeing looks like.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_shape_layer.dart'
    show ShapeSpace;
import 'package:lumit_flutter/panels/viewer_type.dart';
// The width estimate lives with the other "how big is a layer" answers now
// (K-230): the same sum places the caret, the anchor and the wireframe.
import 'package:lumit_flutter/state/layer_bounds.dart';

void main() {
  // The Type tool places a click through the shared comp space now, not a
  // conversion of its own; the sums under test are ShapeSpace.ofComp's.
  (double, double) compPointOf(Offset screen, Rect fitted, Size comp) =>
      ShapeSpace.ofComp(fitted: fitted, compSize: comp).ofScreen(screen);
  group('Where a click lands', () {
    test('a point on the picture becomes a point in the comp', () {
      // A 1920×1080 comp drawn at half size, 100 across and 50 down the panel.
      const fitted = Rect.fromLTWH(100, 50, 960, 540);
      const comp = Size(1920, 1080);
      expect(compPointOf(const Offset(100, 50), fitted, comp), (0.0, 0.0));
      expect(compPointOf(const Offset(1060, 590), fitted, comp),
          (1920.0, 1080.0));
      expect(compPointOf(const Offset(580, 320), fitted, comp), (960.0, 540.0));
    });

    test('the magnification is undone, not assumed', () {
      // The same comp at four times the size: a hundred screen pixels across
      // is twenty-five comp pixels.
      const fitted = Rect.fromLTWH(0, 0, 7680, 4320);
      const comp = Size(1920, 1080);
      final (x, y) = compPointOf(const Offset(100, 100), fitted, comp);
      expect(x, closeTo(25, 1e-9));
      expect(y, closeTo(25, 1e-9));
    });
  });

  group('How wide a line is reckoned to be', () {
    test('half the point size per character — the engine\'s own estimate', () {
      expect(estimatedTextWidth('', 72), 0);
      expect(estimatedTextWidth('Text', 72), 4 * 36);
      expect(estimatedTextWidth('ab', 10), 10);
    });

    test('it counts characters, not bytes', () {
      // A single non-Latin character is one character wide, not its byte count
      // — the caret would otherwise run away on any accented word.
      expect(estimatedTextWidth('é', 40), estimatedTextWidth('e', 40));
    });
  });

  /// The box the Viewer draws round a line of text (K-230). It used to be the
  /// whole composition — text had no measured bounds and the comp was the
  /// fallback — so a click with the Type tool put a box the size of the frame
  /// round twelve-pixel text.
  group('How big a text layer is', () {
    test('as tall as the point size, and no taller', () {
      expect(textLayerBounds('Text', 72).height, 72);
      expect(textLayerBounds('Text', 12).height, 12);
    });

    test('as wide as the line is reckoned to be', () {
      expect(textLayerBounds('Text', 72).width, estimatedTextWidth('Text', 72));
    });

    test('an empty line still has a box, one character wide', () {
      final empty = textLayerBounds('', 12);
      expect(empty.height, 12, reason: 'it says what size it will be set at');
      expect(empty.width, greaterThan(0),
          reason: 'a layer waiting to be typed into must still be visible');
    });
  });

  group('Where a new text layer is anchored', () {
    test('the middle of the estimated line, so it turns about itself', () {
      final anchor = textAnchor('Text', 72);
      expect(anchor.dx, estimatedTextWidth('Text', 72) / 2);
      expect(anchor.dy, 36);
    });

    test('an empty line is anchored on its own left end', () {
      expect(textAnchor('', 72).dx, 0);
    });
  });
}
