// The Ctrl+Space console (popover face from the 2026-08-30 boards):
// what the search ranks, what the category strip narrows, and what the keys
// do while the popover is up.

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/fx_console_context.dart';
import 'package:lumit_flutter/shell/fx_console_frb.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

void main() {
  FxConsoleEntry effect(String label,
          {VoidCallback? run, String group = 'Blur & sharpen'}) =>
      FxConsoleEntry(
        label: label,
        kind: FxConsoleKind.effect,
        group: group,
        run: run ?? () {},
      );
  FxConsoleEntry comp(String label, {VoidCallback? run}) => FxConsoleEntry(
        label: label,
        kind: FxConsoleKind.composition,
        run: run ?? () {},
      );

  group('the search', () {
    test('an earlier, tighter match ranks first', () {
      final entries = [effect('Directional blur'), effect('Blur the edges')];
      final ranked = fxConsoleMatches(entries, 'blur');
      expect(ranked.first.label, 'Blur the edges');
    });
  });

  group('the console widget', () {
    Widget host(FxConsoleModel model, {void Function(BuildContext)? capture}) =>
        Directionality(
          textDirection: TextDirection.ltr,
          child: ThemeScope(
            theme: LumitTheme.dark(),
            animationLevel: AnimationLevel.none,
            showTooltips: false,
            child: Overlay(
              initialEntries: [
                OverlayEntry(
                  builder: (context) {
                    capture?.call(context);
                    return const SizedBox.expand();
                  },
                ),
              ],
            ),
          ),
        );

    Future<void> open(WidgetTester tester, FxConsoleModel model,
        {Offset? anchor}) async {
      late BuildContext ctx;
      await tester.pumpWidget(host(model, capture: (c) => ctx = c));
      showFxConsoleFrb(context: ctx, model: model, anchor: anchor);
      await tester.pump();
      await tester.pump();
    }

    Finder query() => find.byKey(const ValueKey('fx-console-query'));
    Finder item(String label) =>
        find.byKey(ValueKey<String>('fx-console-item-$label'));

    /// The list and strip rebuild around the field as the query narrows them;
    /// the field itself must survive those rebuilds, or its text-input
    /// connection dies and typing stops after one letter.
    /// The second letter is delivered through the **connection already
    /// open**, not via `enterText`, which re-attaches one and would hide
    /// exactly that fault.
    testWidgets('typing keeps going while the list narrows', (tester) async {
      await open(
        tester,
        FxConsoleModel(entries: [effect('Glow'), effect('Gaussian blur')]),
      );
      final field = tester.state<EditableTextState>(find.byType(EditableText));

      await tester.enterText(query(), 'g');
      await tester.pumpAndSettle();
      expect(tester.state<EditableTextState>(find.byType(EditableText)),
          same(field),
          reason: 'the field must survive the rebuild, not be replaced');

      tester.testTextInput.updateEditingValue(const TextEditingValue(
        text: 'ga',
        selection: TextSelection.collapsed(offset: 2),
      ));
      await tester.pumpAndSettle();
      expect(item('Gaussian blur'), findsOneWidget,
          reason: 'the second letter reached the box and narrowed the list');
    });

    testWidgets('typing narrows and Enter applies the top match',
        (tester) async {
      var applied = '';
      await open(
        tester,
        FxConsoleModel(
          entries: [
            effect('Gaussian blur', run: () => applied = 'gaussian'),
            effect('Directional blur', run: () => applied = 'directional'),
          ],
        ),
      );

      await tester.enterText(query(), 'gau');
      await tester.pumpAndSettle();
      expect(item('Gaussian blur'), findsOneWidget);
      expect(item('Directional blur'), findsNothing,
          reason: 'the query narrowed the list');
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(applied, 'gaussian');
    });

    testWidgets('the arrows skip the headings, landing entry to entry',
        (tester) async {
      var applied = '';
      await open(
        tester,
        FxConsoleModel(entries: [
          effect('Glow', run: () => applied = 'glow', group: 'Stylise'),
          effect('Gaussian blur', run: () => applied = 'blur', group: 'Blur'),
        ]),
      );
      // One step down crosses the Blur heading; Enter must run the row under
      // it, never the heading.
      await tester.sendKeyEvent(LogicalKeyboardKey.arrowDown);
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(applied, 'blur');
    });

    testWidgets('the category strip narrows the list, and All lets it back',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(entries: [
          effect('Glow', group: 'Stylise'),
          effect('Gaussian blur', group: 'Blur'),
          comp('Scene 2'),
        ]),
      );
      expect(find.byKey(const ValueKey('fx-console-cat-Stylise')),
          findsOneWidget);

      await tester.tap(find.byKey(const ValueKey('fx-console-cat-Stylise')));
      await tester.pumpAndSettle();
      expect(item('Glow'), findsOneWidget);
      expect(item('Gaussian blur'), findsNothing,
          reason: 'another category\'s row is out');
      expect(item('Scene 2'), findsNothing,
          reason: 'a comp has no category, so a narrowed strip hides it');
      expect(find.byKey(const ValueKey('fx-console-head-Stylise')),
          findsOneWidget, reason: 'the chosen group keeps its heading');
      expect(find.byKey(const ValueKey('fx-console-head-Blur')), findsNothing,
          reason: 'a filtered-out group takes its heading with it');

      await tester.tap(find.byKey(const ValueKey('fx-console-cat-*all')));
      await tester.pumpAndSettle();
      expect(item('Gaussian blur'), findsOneWidget);
      expect(item('Scene 2'), findsOneWidget);
    });
  });

  group('where a snapshot goes', () {
    test('beside the saved project, in a Snapshots folder', () {
      final path = snapshotPathFor(
        compName: 'Scene',
        projectPath: '/work/film/film.lum',
        environment: const {'HOME': '/home/someone'},
      );
      expect(path, contains('/work/film'));
      expect(path, contains('Snapshots'));
      expect(path, endsWith('Scene.png'));
    });

    test('a name a file system cannot take is cleaned, never empty', () {
      expect(
        snapshotPathFor(
            compName: 'Shot 1: "hero"/final',
            environment: const {'HOME': '/h'}),
        endsWith('Shot 1 herofinal.png'),
      );
      expect(
        snapshotPathFor(compName: '///', environment: const {'HOME': '/h'}),
        endsWith('snapshot.png'),
        reason: 'a name that cleans to nothing still needs a file name',
      );
    });
  });
}
