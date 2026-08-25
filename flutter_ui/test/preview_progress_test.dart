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
    );

void main() {
  testWidgets('a frame that arrives quickly never shows a bar',
      (tester) async {
    final tracker = PreviewProgressTracker();
    addTearDown(tracker.dispose);

    tracker.report(_report(12));
    expect(tracker.visible, isFalse, reason: 'nothing shows immediately');

    // Finished well inside the delay: the picture is already there.
    await tester.pump(const Duration(milliseconds: 40));
    tracker.report(_report(12, done: true));
    await tester.pump(PreviewProgressTracker.appearsAfter);
    expect(tracker.visible, isFalse);
  });

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

  testWidgets('playback takes the bar away', (tester) async {
    final tracker = PreviewProgressTracker();
    addTearDown(tracker.dispose);

    tracker.report(_report(3));
    await tester.pump(PreviewProgressTracker.appearsAfter);
    expect(tracker.visible, isTrue);

    // What `play()` calls: whatever was being waited on is not what is being
    // watched now, and playback draws no bar at all.
    tracker.stop();
    expect(tracker.visible, isFalse);
    expect(tracker.frame, isNull);

    // And a stale report cannot bring it back on its own without a fresh
    // wait — the delay starts again.
    tracker.report(_report(3));
    expect(tracker.visible, isFalse);
    await tester.pump(PreviewProgressTracker.appearsAfter);
    expect(tracker.visible, isTrue);
  });

  testWidgets('a run of quick frames — a value drag — stays silent',
      (tester) async {
    final tracker = PreviewProgressTracker();
    addTearDown(tracker.dispose);

    for (var frame = 0; frame < 10; frame++) {
      tracker.report(_report(frame));
      await tester.pump(const Duration(milliseconds: 30));
      tracker.report(_report(frame, done: true));
      expect(tracker.visible, isFalse);
    }
    await tester.pump(PreviewProgressTracker.appearsAfter);
    expect(tracker.visible, isFalse);
  });

  test('every stage has a word for itself, and an unknown one still does', () {
    expect(previewStageLabel(0), 'Preparing');
    expect(previewStageLabel(1), 'Reading media');
    expect(previewStageLabel(3), 'Compositing');
    expect(previewStageLabel(99), 'Rendering');
  });
}
