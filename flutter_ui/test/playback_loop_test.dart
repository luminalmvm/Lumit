import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/playback_loop.dart';

void main() {
  test('a narrowed comp loops the work area', () {
    expect(
      playbackLoop(workStart: 40, workEnd: 90, playhead: 40, lastFrame: 300),
      (start: 40, end: 90),
    );
  });

  test('a comp nobody has narrowed loops the whole of itself', () {
    // The regression: a null work area used to mean "no loop", so two comps in
    // one project played differently — the one somebody had pressed B in
    // looped, the one nobody had touched ran off the end and stopped.
    expect(
      playbackLoop(
          workStart: null, workEnd: null, playhead: 10, lastFrame: 300),
      (start: 0, end: 300),
    );
  });

  test('parked before the work area previews from there and joins the loop',
      () {
    expect(
      playbackLoop(workStart: 40, workEnd: 90, playhead: 5, lastFrame: 300),
      (start: 40, end: 90),
    );
  });

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
