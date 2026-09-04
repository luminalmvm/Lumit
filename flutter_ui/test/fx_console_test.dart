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
    test('matches a subsequence, so "gau" finds Gaussian blur', () {
      expect(fxConsoleScore('gau', 'Gaussian blur'), isNotNull);
      expect(fxConsoleScore('gb', 'Gaussian blur'), isNotNull,
          reason: 'the initials of the two words are a subsequence');
      expect(fxConsoleScore('zzz', 'Gaussian blur'), isNull);
    });

    test('an earlier, tighter match ranks first', () {
      final entries = [effect('Directional blur'), effect('Blur the edges')];
      final ranked = fxConsoleMatches(entries, 'blur');
      expect(ranked.first.label, 'Blur the edges');
    });

    test('effects always come before compositions, however they score', () {
      // The comp is a perfect prefix match; the effect is a scattered one.
      final entries = [comp('Blur'), effect('Directional blur')];
      final ranked = fxConsoleMatches(entries, 'blur');
      expect(ranked.map((e) => e.kind).toList(),
          [FxConsoleKind.effect, FxConsoleKind.composition],
          reason: 'a comp must never outrank an effect');
    });

    test('an empty query keeps the declared order within each kind', () {
      final entries = [comp('Scene'), effect('Invert'), effect('Glow')];
      final ranked = fxConsoleMatches(entries, '');
      expect(ranked.map((e) => e.label).toList(), ['Invert', 'Glow', 'Scene']);
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

    testWidgets('the console opens straight to the popover, list and all',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(entries: [effect('Glow'), comp('Scene 2')]),
      );
      expect(find.byKey(const ValueKey('fx-console-bar')), findsOneWidget);
      expect(item('Glow'), findsOneWidget,
          reason: 'the list is the offer, open from the first frame');
      expect(item('Scene 2'), findsOneWidget);
    });

    testWidgets('the search field holds focus for the console whole life',
        (tester) async {
      await open(tester, FxConsoleModel(entries: [effect('Glow')]));
      expect(tester.binding.focusManager.primaryFocus?.debugLabel,
          'fx-console-query',
          reason: 'typing lands in the box from the first keystroke');

      // Something steals focus: the console takes it straight back, so a
      // stray click can never leave keystrokes falling on the panels.
      tester.binding.focusManager.primaryFocus?.unfocus();
      await tester.pump();
      expect(tester.binding.focusManager.primaryFocus?.debugLabel,
          'fx-console-query',
          reason: 'the console owns the keyboard while it is open');
    });

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

    /// Owner ruling (2026-08-31): no trailing per-row category kicker — the
    /// results group under small inline kicker headings, the Settings-nav
    /// grammar, and comps file under a Compositions heading of their own.
    testWidgets('results group under inline headings, one per group',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(entries: [
          effect('Glow', group: 'Stylise'),
          effect('Gaussian blur', group: 'Blur'),
          effect('Inner glow', group: 'Stylise'),
          comp('Scene 2'),
        ]),
      );
      for (final head in ['Stylise', 'Blur', 'Compositions']) {
        expect(find.byKey(ValueKey<String>('fx-console-head-$head')),
            findsOneWidget,
            reason: 'one heading per group, however its rows ranked');
      }
      expect(find.text('Stylise'), findsOneWidget,
          reason: 'the strip only — the per-row trailing kicker is gone');
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

    testWidgets('Escape retreats one step at a time: clear, then close',
        (tester) async {
      await open(tester, FxConsoleModel(entries: [effect('Glow')]));
      await tester.enterText(query(), 'gl');
      await tester.pumpAndSettle();

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(query(), findsOneWidget, reason: 'still open — the text cleared');

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(query(), findsNothing, reason: 'a second Escape closes');
    });

    testWidgets('the popover opens with its search row on the anchor',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(entries: [effect('Glow')]),
        anchor: const Offset(500, 300),
      );
      final at = tester.getTopLeft(find.byKey(const ValueKey('fx-console-bar')));
      expect(at.dx, 500 - 320 / 2, reason: 'centred on the pointer');
      expect(at.dy, 300 - 28 / 2, reason: 'the search row is under the hand');
    });

    testWidgets('an anchor off the edge is pulled in so the popover fits',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(entries: [effect('Glow')]),
        anchor: const Offset(4, 4),
      );
      final at = tester.getTopLeft(find.byKey(const ValueKey('fx-console-bar')));
      expect(at.dx, greaterThanOrEqualTo(8));
      expect(at.dy, greaterThanOrEqualTo(8));
    });

    testWidgets('the footer sentence draws when the caller has one',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(entries: [effect('Glow')], footer: 'Enter applies'),
      );
      expect(find.byKey(const ValueKey('fx-console-foot')), findsOneWidget);
    });

    testWidgets('no footer sentence, no foot at all', (tester) async {
      await open(tester, FxConsoleModel(entries: [effect('Glow')]));
      expect(find.byKey(const ValueKey('fx-console-foot')), findsNothing);
    });

    testWidgets('the snapshot button saves when there is something to save',
        (tester) async {
      var shots = 0;
      await open(
        tester,
        FxConsoleModel(
          entries: [effect('Glow')],
          onSnapshot: () => shots++,
        ),
      );
      await tester.tap(find.byKey(const ValueKey('fx-console-snapshot')));
      await tester.pumpAndSettle();
      expect(shots, 1);
    });

    testWidgets('the snapshot button greys out with no composition open',
        (tester) async {
      await open(tester, FxConsoleModel(entries: [effect('Glow')]));
      final button = tester.widget<HouseButton>(
          find.byKey(const ValueKey('fx-console-snapshot')));
      expect(button.onPressed, isNull,
          reason: 'no composition, nothing to snapshot');
    });

    /// **The strip is never truncated** (owner item: every category kicker
    /// shows). The popover's width is the board's 320 as a floor, grown to fit
    /// the whole strip; past the 720 cap the strip scrolls sideways rather
    /// than clipping — either way no kicker is cut and nothing overflows.
    testWidgets('the popover grows to fit every category kicker',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(entries: [
          for (final group in [
            'Blur & sharpen',
            'Colour',
            'Distortion',
            'Generate',
            'Stylise',
          ])
            effect('$group thing', group: group),
        ]),
        anchor: const Offset(400, 300),
      );
      final bar = tester.getRect(find.byKey(const ValueKey('fx-console-bar')));
      expect(bar.width, greaterThan(320),
          reason: 'five real groupings need more than the board\'s floor');
      for (final group in ['Blur & sharpen', 'Distortion', 'Stylise']) {
        expect(find.byKey(ValueKey<String>('fx-console-cat-$group')),
            findsOneWidget, reason: 'every kicker is on screen, whole');
      }
      // No RenderFlex overflow: the test framework rethrows one as a failure,
      // so reaching this line is the assertion.
    });

    testWidgets('a catalogue past the cap scrolls the strip instead',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(entries: [
          for (var i = 0; i < 24; i++)
            effect('Effect $i', group: 'A rather long grouping name $i'),
        ]),
      );
      final bar = tester.getRect(find.byKey(const ValueKey('fx-console-bar')));
      expect(bar.width, 720, reason: 'the sane max holds');
      // The strip scrolls; nothing threw, nothing was clipped away silently.
      expect(find.byKey(const ValueKey('fx-console-cat-*all')), findsOneWidget);
    });

    testWidgets('a click beside the popover closes it', (tester) async {
      await open(tester, FxConsoleModel(entries: [effect('Glow')]),
          anchor: const Offset(400, 300));
      await tester.tapAt(const Offset(20, 580));
      await tester.pumpAndSettle();
      expect(query(), findsNothing);
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

    test('an unsaved project goes to the pictures folder, never the cwd', () {
      final path = snapshotPathFor(
        compName: 'Scene',
        environment: const {'HOME': '/home/someone'},
      );
      expect(path, startsWith('/home/someone'));
      expect(path, contains('Pictures'));
      expect(path, isNot(startsWith('Scene')),
          reason: 'a bare name would land wherever the app was started');
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
