// The Type tool's arithmetic: where a click falls in the composition, which
// gap between two letters it lands in, and what a double-click selects.
//
// How wide a line is, and where its letters sit, are the engine's answers now
// (`measureText`), so those are pinned against the real engine in
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
    // Three letters of unequal width, as a real font sets them, and a second
    // line of two under them. The break between the lines is character 3.
    final block = BridgeTextBlock(
      width: 100,
      height: 110,
      ascent: 45,
      descent: 10,
      left: 0,
      right: 100,
      lines: [
        BridgeTextBlockLine(
          start: 0,
          baseline: 40,
          carets: Float64List.fromList([0, 40, 50, 100]),
        ),
        BridgeTextBlockLine(
          start: 4,
          baseline: 100,
          carets: Float64List.fromList([0, 30, 60]),
        ),
      ],
    );
    // Halfway up the first line's letters.
    Offset first(double x) => Offset(x, 22);

    test('the nearest gap, not the nearest letter', () {
      expect(caretNearest(block, first(-20)), 0, reason: 'before the line');
      expect(caretNearest(block, first(19)), 0);
      expect(caretNearest(block, first(21)), 1);
      expect(caretNearest(block, first(46)), 2, reason: 'the narrow letter');
      expect(caretNearest(block, first(76)), 3);
      expect(caretNearest(block, first(400)), 3, reason: 'past the end');
    });

    test('a click on the second line counts on from the first', () {
      expect(caretNearest(block, const Offset(-5, 85)), 4);
      expect(caretNearest(block, const Offset(32, 85)), 5);
      expect(caretNearest(block, const Offset(500, 300)), 6,
          reason: 'below and past the last line');
    });

    test('a caret knows its line, and the break stays on the line it ends',
        () {
      expect(caretPlace(block, 1), (line: 0, x: 40.0));
      expect(caretPlace(block, 3), (line: 0, x: 100.0));
      expect(caretPlace(block, 4), (line: 1, x: 0.0));
      expect(caretPlace(block, 6), (line: 1, x: 60.0));
      expect(caretPlace(block, 99), (line: 1, x: 60.0));
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
