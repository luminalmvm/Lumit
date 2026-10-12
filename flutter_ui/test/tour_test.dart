// The guided tour: a step with nothing on screen to point at is passed over,
// and a tour that has been finished does not open by itself again.
//
// Both are once-only or never-visible failures. A card pointing at nothing is
// what a new user sees on a window too narrow for a panel, and a tour that
// came back on the second launch would be worse than not having one.

import 'dart:io';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/tour.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/escape_ladder.dart';

void main() {
  setUp(() {
    // Never the developer's own settings file, and never one a run before
    // this left behind.
    Workspace.storeOverride =
        '${Directory.systemTemp.createTempSync('lumit-tour').path}/workspace.json';
  });

  tearDown(() => Workspace.storeOverride = null);

  TourStep step(String name) => TourStep(
        target: name,
        title: 'Step $name',
        body: 'About $name.',
        side: TourSide.right,
      );

  /// A window with one keyed box for each name in [on], as it stands, and a
  /// place to raise the tour from.
  Widget host(ValueNotifier<List<String>> on) => Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Overlay(
            initialEntries: [
              OverlayEntry(
                builder: (_) => Align(
                  alignment: Alignment.topLeft,
                  child: ValueListenableBuilder<List<String>>(
                    valueListenable: on,
                    builder: (context, names, _) => Column(
                      key: const ValueKey('shell'),
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        for (final name in names)
                          SizedBox(
                              key: ValueKey<String>(name),
                              width: 80,
                              height: 40),
                      ],
                    ),
                  ),
                ),
              ),
            ],
          ),
        ),
      );

  testWidgets(
      'a missing target is passed over, and finishing stops it reopening',
      (tester) async {
    // A machine with no settings file, which is the one the tour opens on.
    final workspace = Workspace()..load();
    expect(workspace.tourDone, isFalse);
    final steps = [step('a'), step('gone'), step('c')];
    BuildContext shell() => tester.element(find.byKey(const ValueKey('shell')));

    final on = ValueNotifier(['a', 'c']);
    await tester.pumpWidget(host(on));
    expect(showTour(shell(), workspace, steps: steps), isTrue);
    await tester.pump();
    expect(find.text('Step a'), findsOneWidget);
    expect(lumitModalOpen, isTrue, reason: 'the panels stand down under it');

    // Next goes past the step with nothing to point at, and that leaves the
    // one it lands on as the last.
    await tester.tap(find.byKey(const ValueKey('tour-next')));
    await tester.pump();
    expect(find.text('Step gone'), findsNothing);
    expect(find.text('Step c'), findsOneWidget);
    expect(find.text('Done'), findsOneWidget);

    await tester.tap(find.byKey(const ValueKey('tour-next')));
    await tester.pump();
    expect(find.byKey(const ValueKey('tour-card')), findsNothing);
    // Written down, so the next launch reads it and leaves the tour to Help.
    expect((Workspace()..load()).tourDone, isTrue);

    // Asked for, as Help does, and then the window loses everything the
    // tour could point at: it ends, and leaves nothing of itself behind.
    expect(showTour(shell(), workspace, steps: steps), isTrue);
    await tester.pump();
    expect(find.text('Step a'), findsOneWidget);
    on.value = const [];
    await tester.pump();
    await tester.pump();
    expect(find.byKey(const ValueKey('tour-card')), findsNothing);
    expect(lumitModalOpen, isFalse);
    expect(EscapeLadder.press(), isFalse,
        reason: 'the tour let go of Escape when it left');
  });
}
