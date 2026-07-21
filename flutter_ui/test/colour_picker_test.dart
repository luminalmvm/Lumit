// The colour-picker conversion maths, pinned both ways: an HSV↔RGB table and
// hex parse/format round-trips (including '#' tolerance and rejection of bad
// input). These are pure functions, so a drift here would silently miscolour
// every pick.

import 'dart:ui';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/widgets/colour_picker.dart';

int r(Color c) => (c.r * 255).round();
int g(Color c) => (c.g * 255).round();
int b(Color c) => (c.b * 255).round();

void main() {
  group('hsvToRgb', () {
    void expectRgb(Hsv hsv, int er, int eg, int eb) {
      final c = hsvToRgb(hsv.$1, hsv.$2, hsv.$3);
      expect([r(c), g(c), b(c)], [er, eg, eb], reason: '$hsv');
    }

    test('the conversion table (HSV → RGB)', () {
      expectRgb((0, 0, 0), 0, 0, 0); // black
      expectRgb((0, 0, 1), 255, 255, 255); // white
      expectRgb((0, 1, 1), 255, 0, 0); // pure red
      expectRgb((120, 1, 1), 0, 255, 0); // pure green
      expectRgb((240, 1, 1), 0, 0, 255); // pure blue
      expectRgb((120, 0.5, 0.8), 102, 204, 102); // mid sat / mid value
    });

    test('the alpha channel is always opaque', () {
      final c = hsvToRgb(200, 0.4, 0.6);
      expect((c.a * 255).round(), 0xff);
    });
  });

  group('rgbToHsv', () {
    void expectHsv(int cr, int cg, int cb, double eh, double es, double ev) {
      final hsv = rgbToHsv(Color.fromARGB(0xff, cr, cg, cb));
      expect(hsv.$1, closeTo(eh, 1e-6), reason: 'hue');
      expect(hsv.$2, closeTo(es, 1e-6), reason: 'saturation');
      expect(hsv.$3, closeTo(ev, 1e-6), reason: 'value');
    }

    test('the conversion table (RGB → HSV)', () {
      expectHsv(0, 0, 0, 0, 0, 0); // black
      expectHsv(255, 255, 255, 0, 0, 1); // white
      expectHsv(255, 0, 0, 0, 1, 1); // pure red
      expectHsv(0, 255, 0, 120, 1, 1); // pure green
      expectHsv(0, 0, 255, 240, 1, 1); // pure blue
      expectHsv(102, 204, 102, 120, 0.5, 0.8); // mid sat / mid value
    });

    test('round-trips back through hsvToRgb', () {
      for (final sample in [
        const Color.fromARGB(0xff, 12, 200, 90),
        const Color.fromARGB(0xff, 200, 40, 160),
        const Color.fromARGB(0xff, 224, 90, 114), // the default clay accent
      ]) {
        final hsv = rgbToHsv(sample);
        final back = hsvToRgb(hsv.$1, hsv.$2, hsv.$3);
        expect([r(back), g(back), b(back)],
            [r(sample), g(sample), b(sample)]);
      }
    });
  });

  group('hex parse/format', () {
    test('parses six digits, tolerating a leading #', () {
      final a = parseHex('e05a72');
      final b0 = parseHex('#E05A72');
      expect(a, isNotNull);
      expect(b0, isNotNull);
      expect([r(a!), g(a), b(a)], [0xe0, 0x5a, 0x72]);
      expect([r(b0!), g(b0), b(b0)], [0xe0, 0x5a, 0x72]);
    });

    test('trims surrounding whitespace', () {
      final c = parseHex('  ff8800  ');
      expect(c, isNotNull);
      expect([r(c!), g(c), b(c)], [0xff, 0x88, 0x00]);
    });

    test('rejects malformed input', () {
      expect(parseHex(''), isNull);
      expect(parseHex('12345'), isNull); // too short
      expect(parseHex('1234567'), isNull); // too long
      expect(parseHex('gg0000'), isNull); // non-hex
      expect(parseHex('#12g456'), isNull);
      expect(parseHex('not a colour'), isNull);
    });

    test('formats as upper-case RRGGBB with no #', () {
      expect(formatHex(const Color.fromARGB(0xff, 0xe0, 0x5a, 0x72)), 'E05A72');
      expect(formatHex(const Color.fromARGB(0xff, 0, 0, 0)), '000000');
      expect(formatHex(const Color.fromARGB(0xff, 255, 136, 0)), 'FF8800');
    });

    test('round-trips through parse and format', () {
      for (final s in ['000000', 'FFFFFF', 'E05A72', '1A2B3C', 'FF8800']) {
        expect(formatHex(parseHex(s)!), s);
      }
    });
  });
}
