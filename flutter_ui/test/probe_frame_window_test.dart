// The probe's fps arithmetic, pinned (docs/impl/ui-performance.md §6).
//
// The probe once counted whatever FrameTimings landed in its bucket over a
// window that closed 300 ms after the gesture, and divided by a wall clock
// that included that tail. That reads every gesture ~15% slow, loses frames
// the engine's timings batch (flushed up to a second late) had not delivered,
// and cannot compute per-frame gaps — which is what hid the truth of the
// "12 fps" scroll row: frames tracked wheel notches one for one, and the
// missing frames were notches grinding against the scroll extent's stops,
// where drawing nothing is correct. The fix selects frames by their vsync
// timestamp against the gesture's own window; this test fails if that
// selection ever drifts back to bucket membership.
import 'dart:ui';

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/probe/perf_probe.dart';

FrameTiming frameAt(int vsyncUs) => FrameTiming(
      vsyncStart: vsyncUs,
      buildStart: vsyncUs + 100,
      buildFinish: vsyncUs + 3000,
      rasterStart: vsyncUs + 3100,
      rasterFinish: vsyncUs + 9000,
      rasterFinishWallTime: vsyncUs + 9000,
    );

void main() {
  test('fps counts the frames whose vsync falls inside the gesture window, '
      'however late their timings report arrived', () {
    const t0 = 1000000; // gesture start, µs
    const t1 = 2500000; // gesture end
    final frames = <FrameTiming>[
      // Landed in the bucket during the pre-gesture settle: not the gesture's.
      frameAt(t0 - 40000),
      // The gesture's own frames — in a real run the last batch of these
      // arrives after the gesture, in a late timings report. Their
      // timestamps place them; where the report landed must not matter.
      for (var v = t0; v < t1; v += 100000) frameAt(v),
      // Trailing follow-on frames after the gesture: reported, not counted.
      frameAt(t1 + 1000),
      frameAt(t1 + 200000),
    ];
    final counted = framesWithin(frames, t0, t1);
    expect(counted, hasLength(15));
    expect(
      counted.map(
          (f) => f.timestampInMicroseconds(FramePhase.vsyncStart) >= t0 &&
              f.timestampInMicroseconds(FramePhase.vsyncStart) < t1),
      everyElement(isTrue),
    );
    // The fps the probe prints is count over the gesture's own wall clock —
    // 15 frames over 1.5 s — and no tail it waits out can dilute it.
    expect(counted.length / ((t1 - t0) / 1e6), closeTo(10.0, 0.001));
  });

  test('an empty window counts nothing', () {
    expect(framesWithin([frameAt(500)], 1000, 1000), isEmpty);
  });
}
