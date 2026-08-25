// The shared clock face, and the two things K-287 added to it: how wide a
// timecode is at a given rate, and a timecode that can be negative (a Retime
// asking for a moment before the start of its media).

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/timecode.dart';

void main() {
  test('the frames field widens with the rate, and the slot with it', () {
    expect(timecodeFrameDigits(24, 1), 2);
    expect(timecodeFrameDigits(60, 1), 2);
    expect(timecodeFrameDigits(120, 1), 3);
    expect(timecodeFrameDigits(600, 1), 3);
    // `HH:MM:SS:` is nine characters, then the frames field.
    expect(timecodeChars(24, 1), 11);
    expect(timecodeChars(600, 1), 12);
  });

  test('a rate of nothing still gives a slot to draw in', () {
    expect(timecodeChars(0, 0), 11);
  });

  test('a source time before zero reads with a minus sign', () {
    expect(timecodeOfRateSigned(0, 24, 1), '00:00:00:00');
    expect(timecodeOfRateSigned(25, 24, 1), '00:00:01:01');
    expect(timecodeOfRateSigned(-25, 24, 1), '-00:00:01:01');
  });

  test('and reads back, sign and all', () {
    expect(framesOfTimecodeSigned('00:00:01:01', 24, 1), 25);
    expect(framesOfTimecodeSigned('-00:00:01:01', 24, 1), -25);
    expect(framesOfTimecodeSigned(' -00:00:01:01 ', 24, 1), -25);
    expect(framesOfTimecodeSigned('later', 24, 1), isNull);
    expect(framesOfTimecodeSigned('-', 24, 1), isNull);
  });

  test('a signed timecode round-trips at an inexact rate', () {
    for (final frame in [0, 1, 29, 30, 899, -1, -30, -1801]) {
      final shown = timecodeOfRateSigned(frame, 30000, 1001);
      expect(framesOfTimecodeSigned(shown, 30000, 1001), frame,
          reason: 'frame $frame reads back from $shown');
    }
  });
}
