import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/state/playback_loop.dart';

void main() {
  test('a narrowed comp loops the work area', () {
    expect(
      playbackLoop(workStart: 40, workEnd: 90, playhead: 40),
      (start: 40, end: 90),
    );
  });

  test('parked before the work area previews from there and joins the loop',
      () {
    expect(
      playbackLoop(workStart: 40, workEnd: 90, playhead: 5),
      (start: 40, end: 90),
    );
  });

  test('parked past the work area previews the tail instead of snapping back',
      () {
    // The regression: the first frame to arrive was already past the end, so
    // the loop pulled the playhead back inside before anything had been seen.
    expect(playbackLoop(workStart: 40, workEnd: 90, playhead: 91), isNull);
    // The end frame itself is still inside the span, and still loops.
    expect(
      playbackLoop(workStart: 40, workEnd: 90, playhead: 90),
      (start: 40, end: 90),
    );
  });

  test('a span with no room in it does not loop', () {
    // Would otherwise restart on every frame that arrived.
    expect(playbackLoop(workStart: 40, workEnd: 40, playhead: 40), isNull);
  });
}
