// The Viewer's preview progress bar decides one thing: when there is something
// to draw. Both halves of that are behaviour worth pinning — a bar that never
// appeared would be pointless, and a bar that appeared for every frame of a
// drag would be worse than none.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:lumit_flutter/state/preview_progress.dart';

BridgeRenderProgress _report(
  int frame, {
  double fraction = 0.3,
  int stage = 1,
  bool done = false,
}) =>
    BridgeRenderProgress(
      frame: BigInt.from(frame),
      stage: stage,
      fraction: fraction,
      done: done,
      view: 0,
    );

void main() {
  testWidgets('a frame worth waiting for shows a bar, and it goes when the '
      'frame lands', (tester) async {
    final tracker = PreviewProgressTracker();
    addTearDown(tracker.dispose);
    var notified = 0;
    tracker.addListener(() => notified++);

    tracker.report(_report(7, fraction: 0.1, stage: 1));
    await tester.pump(PreviewProgressTracker.appearsAfter);
    expect(tracker.visible, isTrue);
    expect(tracker.frame, 7);
    expect(tracker.fraction, closeTo(0.1, 1e-9));
    expect(tracker.label, 'Reading media');
    expect(notified, greaterThan(0), reason: 'the Viewer was told to repaint');

    tracker.report(_report(7, fraction: 0.8, stage: 3));
    expect(tracker.fraction, closeTo(0.8, 1e-9));
    expect(tracker.label, 'Compositing');

    tracker.report(_report(7, done: true));
    expect(tracker.visible, isFalse, reason: 'the frame arrived');
  });
}
