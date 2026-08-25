// The bar itself: it draws nothing until there is a wait worth reporting, it
// says what the engine is doing while there is, and it leaves no animation
// ticking behind it — a widget that never lets the interface settle hangs every
// test that waits for it to (the frame-rate readout learned this first).

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_progress_bar.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:lumit_flutter/state/preview_progress.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

BridgeRenderProgress _report(int frame,
        {double fraction = 0.42, int stage = 3, bool done = false}) =>
    BridgeRenderProgress(
      frame: BigInt.from(frame),
      stage: stage,
      fraction: fraction,
      done: done,
    );

void main() {
  Widget host(PreviewProgressTracker tracker,
          {AnimationLevel animation = AnimationLevel.none}) =>
      Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: animation,
          showTooltips: false,
          // Sized by what it draws, not by the box it is in: the bar sits on
          // the right of the transport now (K-287), taking only the room it
          // needs, and its width is part of what is being tested.
          child: Align(
            alignment: Alignment.topLeft,
            child: ViewerProgressBar(tracker: tracker),
          ),
        ),
      );

  testWidgets('nothing is drawn while nothing is being waited for',
      (tester) async {
    final tracker = PreviewProgressTracker();
    addTearDown(tracker.dispose);
    await tester.pumpWidget(host(tracker));
    expect(find.byType(Text), findsNothing);
  });

  testWidgets('a slow frame draws its stage and how far it has got',
      (tester) async {
    final tracker = PreviewProgressTracker();
    addTearDown(tracker.dispose);
    await tester.pumpWidget(host(tracker));

    tracker.report(_report(11, fraction: 0.42, stage: 3));
    await tester.pump(PreviewProgressTracker.appearsAfter);
    await tester.pump();

    expect(find.text('Compositing'), findsOneWidget);
    expect(find.text('42%'), findsOneWidget);

    // The frame lands: the bar goes, and the interface settles — a pending
    // animation here would make the next line time out.
    tracker.report(_report(11, done: true));
    await tester.pumpAndSettle();
    expect(find.text('Compositing'), findsNothing);
  });

  testWidgets('the sheen animates only while the bar is on screen',
      (tester) async {
    final tracker = PreviewProgressTracker();
    addTearDown(tracker.dispose);
    await tester.pumpWidget(host(tracker, animation: AnimationLevel.all));

    tracker.report(_report(4));
    await tester.pump(PreviewProgressTracker.appearsAfter);
    await tester.pump();
    expect(tester.binding.transientCallbackCount, greaterThan(0),
        reason: 'something is moving while the frame is being waited for');

    tracker.report(_report(4, done: true));
    await tester.pumpAndSettle();
    expect(tester.binding.transientCallbackCount, 0,
        reason: 'and nothing is moving once it has arrived');
  });

  /// The bar rides on the transport now (K-287), where a percentage that
  /// resized itself as it counted would jog every control beside it.
  testWidgets('the bar is the same width at 9% as at 100%', (tester) async {
    final tracker = PreviewProgressTracker();
    addTearDown(tracker.dispose);
    await tester.pumpWidget(host(tracker));

    tracker.report(_report(7, fraction: 0.09, stage: 3));
    await tester.pump(PreviewProgressTracker.appearsAfter);
    await tester.pump();
    final narrow = tester.getSize(find.byType(ViewerProgressBar));

    tracker.report(_report(7, fraction: 1.0, stage: 3));
    await tester.pump();
    expect(tester.getSize(find.byType(ViewerProgressBar)), narrow);

    tracker.report(_report(7, done: true));
    await tester.pumpAndSettle();
  });
}
