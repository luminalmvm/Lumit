// Arithmetic in a value well (Caddis A3).
//
// Half of what a person types into a size or a position is a sum they did in
// their head first — half of this, that less the margin, twice the frame. The
// well does the sum instead. What must not change is everything else about
// typing a number: a plain number reads exactly as it always did, and text
// that is not a sum this understands is refused the way a well has always
// refused text — by keeping the value it has and saying nothing.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/widgets/controls.dart';

void main() {
  group('a value well reads a sum', () {
    test("the owner's own example commits", () {
      expect(parseNumberField('(1920-100)*0.5'), 910);
    });

    test('multiplication binds tighter than addition, as on paper', () {
      expect(parseNumberField('2+3*4'), 14);
      expect(parseNumberField('2*3+4'), 10);
      expect(parseNumberField('100-10/2'), 95);
    });

    test('brackets take precedence back, and nest', () {
      expect(parseNumberField('(2+3)*4'), 20);
      expect(parseNumberField('((2+3)*4)/10'), 2);
    });

    test('a minus in front is a sign, not a mistake', () {
      expect(parseNumberField('-40'), -40);
      expect(parseNumberField('-(2+3)'), -5);
      expect(parseNumberField('10*-2'), -20);
      expect(parseNumberField('10--2'), 12);
    });

    test('spaces are how people type, and are ignored', () {
      expect(parseNumberField(' (1920 - 100) * 0.5 '), 910);
    });
  });

  group('and everything else is unchanged', () {
    test('a plain number is a plain number', () {
      expect(parseNumberField('42'), 42);
      expect(parseNumberField('-1.5'), -1.5);
      expect(parseNumberField(' 7 '), 7);
      // An integer stays an integer: a well with no decimals rounds what it
      // is given, and `42` must not arrive as `42.0` from a different route.
      expect(parseNumberField('42'), isA<int>());
    });

    test('a division by zero is refused, calmly', () {
      expect(parseNumberField('5/0'), isNull);
      expect(parseNumberField('0/0'), isNull);
      expect(parseNumberField('(1+1)/(2-2)'), isNull);
    });

    test('anything that is not a sum is refused too', () {
      expect(parseNumberField(''), isNull);
      expect(parseNumberField('  '), isNull);
      expect(parseNumberField('3+4x'), isNull);
      expect(parseNumberField('(2+3'), isNull);
      expect(parseNumberField('2+'), isNull);
      expect(parseNumberField('*3'), isNull);
      expect(parseNumberField('1.2.3'), isNull);
      expect(parseNumberField('half'), isNull);
    });
  });
}
