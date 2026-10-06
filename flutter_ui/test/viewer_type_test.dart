// The Type tool's arithmetic: where a click falls in the composition, which
// gap between two letters it lands in, and what a double-click selects.
//
// How wide a line is, and where its letters sit, are the engine's answers now
// (`measureTextLine`), so those are pinned against the real engine in
// test/frb/viewer_type_frb_test.dart. What is left here needs no library.

import 'dart:typed_data';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_shape_layer.dart'
    show ShapeSpace;
import 'package:lumit_flutter/panels/viewer_type.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';

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

  group('Which gap a click lands in', () {
    // Three letters of unequal width, as a real font sets them.
    final line = BridgeTextLine(
      width: 100,
      height: 50,
      baseline: 40,
      ascent: 45,
      descent: 10,
      carets: Float64List.fromList([0, 40, 50, 100]),
    );

    test('the nearest gap, not the nearest letter', () {
      expect(caretNearest(line, -20), 0, reason: 'before the line');
      expect(caretNearest(line, 19), 0);
      expect(caretNearest(line, 21), 1);
      expect(caretNearest(line, 46), 2, reason: 'the narrow letter');
      expect(caretNearest(line, 76), 3);
      expect(caretNearest(line, 400), 3, reason: 'past the end');
    });
  });

  group('Characters and UTF-16', () {
    test('the same count until a character takes two units', () {
      expect(characterIndexOf('abc', 2), 2);
      expect(utf16OffsetOf('abc', 2), 2);
    });

    test('an emoji is one character and two units', () {
      const text = 'a\u{1F600}b';
      expect(text.length, 4);
      expect(characterIndexOf(text, 3), 2, reason: 'after the emoji');
      expect(utf16OffsetOf(text, 2), 3);
      expect(utf16OffsetOf(text, 3), 4);
    });
  });

  group('What a double-click selects', () {
    const text = 'Hello, this is';

    test('the word under the click', () {
      expect(wordAround(text, 1),
          const TextSelection(baseOffset: 0, extentOffset: 5));
      expect(wordAround(text, 9),
          const TextSelection(baseOffset: 7, extentOffset: 11));
    });

    test('a run of spaces, or a lone mark, is its own word', () {
      expect(wordAround(text, 5),
          const TextSelection(baseOffset: 5, extentOffset: 6));
      expect(wordAround(text, 6),
          const TextSelection(baseOffset: 6, extentOffset: 7));
    });

    test('past the end is the last word', () {
      expect(wordAround(text, text.length),
          const TextSelection(baseOffset: 12, extentOffset: 14));
    });
  });
}
