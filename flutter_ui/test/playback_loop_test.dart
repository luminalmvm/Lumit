import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/playback_loop.dart';

void main() {
  clockFrameTests();

  test('parked past the work area previews the tail instead of snapping back',
      () {
    // The regression: the first frame to arrive was already past the end, so
    // the loop pulled the playhead back inside before anything had been seen.
    expect(
      playbackLoop(workStart: 40, workEnd: 90, playhead: 91, lastFrame: 300),
      isNull,
    );
    // The end frame itself is still inside the span, and still loops.
    expect(
      playbackLoop(workStart: 40, workEnd: 90, playhead: 90, lastFrame: 300),
      (start: 40, end: 90),
    );
    // The whole-comp fallback has nowhere past its end to be parked.
    expect(
      playbackLoop(
          workStart: null, workEnd: null, playhead: 300, lastFrame: 300),
      (start: 0, end: 300),
    );
  });

  test('a span with no room in it does not loop', () {
    // Would otherwise restart on every frame that arrived.
    expect(
      playbackLoop(workStart: 40, workEnd: 40, playhead: 40, lastFrame: 300),
      isNull,
    );
    // A one-frame comp is the same case through the fallback.
    expect(
      playbackLoop(workStart: null, workEnd: null, playhead: 0, lastFrame: 0),
      isNull,
    );
  });
}

void clockFrameTests() {
  test('the adaptive clock counts on from the last picture at the comp rate',
      () {
    // The regression: adaptive playback only moved the playhead when a picture
    // arrived, so on a comp too heavy for its rate it jumped three or four
    // frames at a time and stood still between.
    expect(
      clockFrame(anchorFrame: 10, sinceAnchorMicros: 0, fps: 25, end: 100),
      10,
    );
    expect(
      clockFrame(anchorFrame: 10, sinceAnchorMicros: 100000, fps: 25, end: 100),
      12,
    );
    // Part-way through a frame period rounds down: the frame has not arrived.
    expect(
      clockFrame(anchorFrame: 10, sinceAnchorMicros: 39000, fps: 25, end: 100),
      10,
    );
  });
}
