// The shared clock face, and the two things added to it later: how wide a
// timecode is at a given rate, and a timecode that can be negative (a Retime
// asking for a moment before the start of its media).

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/timecode.dart';

void main() {
  test('a signed timecode round-trips at an inexact rate', () {
    for (final frame in [0, 1, 29, 30, 899, -1, -30, -1801]) {
      final shown = timecodeOfRateSigned(frame, 30000, 1001);
      expect(framesOfTimecodeSigned(shown, 30000, 1001), frame,
          reason: 'frame $frame reads back from $shown');
    }
    // Drop-frame skips two numbers a minute, so an hour of 29.97 is 107892
    // frames and the first frame of the second minute is numbered 02.
    expect(timecodeOfRate(107892, 30000, 1001, dropFrame: true), '01:00:00;00');
    expect(timecodeOfRate(1800, 30000, 1001, dropFrame: true), '00:01:00;02');
  });
}
