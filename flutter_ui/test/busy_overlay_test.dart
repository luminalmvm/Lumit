// The card the shell puts up for a job that takes seconds on the document it
// already has open — beat detection is the first (docs/09 §5).
//
// The job here is a fake: a Completer stands in for the engine's, so the two
// moments that matter can be held apart — running, and settled. What the test
// is really guarding is that the card comes down on a failure as well as on a
// success, because a job that ends in nothing would otherwise leave the shell
// covered with no way back.

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/splash.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

Widget _harness(Widget child) => Directionality(
      textDirection: TextDirection.ltr,
      child: ThemeScope(
        theme: LumitTheme.dark(),
        animationLevel: AnimationLevel.none,
        showTooltips: false,
        child: child,
      ),
    );

void main() {
  testWidgets('the card stands while the job runs and goes when it is done',
      (tester) async {
    await tester.binding.setSurfaceSize(const Size(800, 600));
    final busy = ValueNotifier<String?>(null);
    addTearDown(busy.dispose);

    await tester.pumpWidget(_harness(BusyOverlay(busy: busy)));
    expect(find.text('Detecting beats'), findsNothing,
        reason: 'nothing is running, so there is no card');

    final job = Completer<void>();
    final shown = showBusyWhile(busy, 'Detecting beats', job.future);
    await tester.pump();
    expect(find.text('Detecting beats'), findsOneWidget);

    job.complete();
    await shown;
    await tester.pump();
    expect(find.text('Detecting beats'), findsNothing,
        reason: 'the job finished, so the card comes down');
  });

  testWidgets('a job that fails takes the card down with it', (tester) async {
    await tester.binding.setSurfaceSize(const Size(800, 600));
    final busy = ValueNotifier<String?>(null);
    addTearDown(busy.dispose);

    await tester.pumpWidget(_harness(BusyOverlay(busy: busy)));

    final job = Completer<void>();
    final shown = showBusyWhile(busy, 'Detecting beats', job.future);
    await tester.pump();
    expect(find.text('Detecting beats'), findsOneWidget);

    job.completeError(StateError('no audio in this composition'));
    await expectLater(shown, throwsStateError);
    await tester.pump();
    expect(find.text('Detecting beats'), findsNothing);
    expect(busy.value, isNull);
  });

  // The same card is what opening a project puts up, and *that* job can say how
  // far it has got (K-628). A fraction fills the bar and writes the percentage
  // beside the line; no fraction keeps the sweep and says no number at all,
  // because a job that reports nothing has no honest one to show.
  testWidgets('a fraction fills the bar and says the percentage',
      (tester) async {
    await tester.binding.setSurfaceSize(const Size(800, 600));

    await tester.pumpWidget(_harness(const OpeningOverlay(fraction: 0.4)));
    await tester.pump();
    expect(find.text('40%'), findsOneWidget);
    expect(
        tester
            .widgetList<HouseProgressBar>(find.byType(HouseProgressBar))
            .single
            .fraction,
        0.4);
  });

  testWidgets('a job that cannot say keeps the sweep and no percentage',
      (tester) async {
    await tester.binding.setSurfaceSize(const Size(800, 600));

    await tester.pumpWidget(_harness(const OpeningOverlay()));
    await tester.pump();
    expect(find.textContaining('%'), findsNothing);
  });
}
