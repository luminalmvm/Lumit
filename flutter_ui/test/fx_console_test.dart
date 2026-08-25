// The Ctrl+Space console (K-324, reshaped by K-325): what the search ranks
// and divides, where the console opens, and what the ring does with a flick —
// including the rings inside rings.

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/shell/fx_console_context.dart';
import 'package:lumit_flutter/shell/fx_console_frb.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/radial_maths.dart';

void main() {
  FxConsoleEntry effect(String label, {VoidCallback? run}) => FxConsoleEntry(
        label: label,
        kind: FxConsoleKind.effect,
        group: 'Blur & sharpen',
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
    Finder centre() => find.byKey(const ValueKey('fx-radial-centre'));

    testWidgets('the search field holds focus for the console whole life',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'Scene',
          radial: [RadialEntry(label: 'Solid', run: () {})],
          entries: [effect('Glow')],
        ),
      );
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

    /// The first letter hides the ring, which changes the Stack's children —
    /// and an unkeyed Stack matches its children by index, so the bar's
    /// element was recycled onto the ring's old slot and the field rebuilt
    /// from nothing. Its text-input connection died with it and typing
    /// stopped dead after one letter (K-328).
    ///
    /// The second letter is delivered through the **connection already
    /// open**, not via `enterText`, which re-attaches one and would hide
    /// exactly this fault.
    testWidgets('typing keeps going after the ring steps aside',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'Scene',
          radial: [RadialEntry(label: 'Solid', run: () {})],
          entries: [effect('Glow'), effect('Gaussian blur')],
        ),
      );
      final field = tester.state<EditableTextState>(find.byType(EditableText));

      await tester.enterText(query(), 'g');
      await tester.pumpAndSettle();
      expect(centre(), findsNothing, reason: 'the ring has stepped aside');
      expect(tester.state<EditableTextState>(find.byType(EditableText)),
          same(field),
          reason: 'the field must survive the ring leaving, not be rebuilt');

      tester.testTextInput.updateEditingValue(const TextEditingValue(
        text: 'ga',
        selection: TextSelection.collapsed(offset: 2),
      ));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('fx-console-item-Gaussian blur')),
          findsOneWidget,
          reason: 'the second letter reached the box and narrowed the list');
    });

    testWidgets('an empty bar lists nothing — the ring is the offer',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'Scene',
          radial: [RadialEntry(label: 'Solid', run: () {})],
          entries: [effect('Glow'), comp('Scene 2')],
        ),
      );
      expect(find.byKey(const ValueKey('fx-console-item-Glow')), findsNothing,
          reason: 'no query, no list — typing is what asks for one');
      expect(find.text('Scene 2'), findsNothing);
      expect(centre(), findsOneWidget, reason: 'the ring is what is offered');
    });

    testWidgets('typing opens the dropdown and Enter applies the top match',
        (tester) async {
      var applied = '';
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'Timeline',
          radial: const [],
          entries: [
            effect('Gaussian blur', run: () => applied = 'gaussian'),
            effect('Directional blur', run: () => applied = 'directional'),
          ],
        ),
      );

      await tester.enterText(query(), 'gau');
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('fx-console-item-Gaussian blur')),
          findsOneWidget,
          reason: 'the dropdown appears under the bar once there is a query');
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(applied, 'gaussian');
    });

    testWidgets('typing lets the ring step aside; clearing brings it back',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'Scene',
          radial: [RadialEntry(label: 'Solid', run: () {})],
          entries: [effect('Glow')],
        ),
      );
      expect(centre(), findsOneWidget);

      await tester.enterText(query(), 'g');
      await tester.pumpAndSettle();
      expect(centre(), findsNothing,
          reason: 'the dropdown needs the room, and typing chose the bar');

      await tester.enterText(query(), '');
      await tester.pumpAndSettle();
      expect(centre(), findsOneWidget, reason: 'an empty bar offers the ring');
    });

    testWidgets('Escape retreats one step at a time: clear, then close',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'Scene',
          radial: [RadialEntry(label: 'Solid', run: () {})],
          entries: [effect('Glow')],
        ),
      );
      await tester.enterText(query(), 'gl');
      await tester.pumpAndSettle();

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(query(), findsOneWidget, reason: 'still open — the text cleared');
      expect(centre(), findsOneWidget, reason: 'and the ring is back');

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(query(), findsNothing, reason: 'a second Escape closes');
    });

    testWidgets('the ring opens centred on the anchor', (tester) async {
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'Scene',
          radial: [RadialEntry(label: 'Solid', run: () {})],
          entries: const [],
        ),
        anchor: const Offset(500, 300),
      );
      expect(tester.getCenter(centre()), const Offset(500, 300),
          reason: 'the flick starts where the pointer already is');
    });

    testWidgets('an anchor off the edge is pulled in so the ring fits',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'Scene',
          radial: [RadialEntry(label: 'Solid', run: () {})],
          entries: const [],
        ),
        anchor: const Offset(4, 4),
      );
      final at = tester.getCenter(centre());
      expect(at.dx, greaterThanOrEqualTo(radialExtent));
      expect(at.dy, greaterThanOrEqualTo(radialExtent));
    });

    testWidgets('the snapshot button saves when there is something to save',
        (tester) async {
      var shots = 0;
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'Scene',
          radial: const [],
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
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'Nothing selected',
          radial: const [],
          entries: [effect('Glow')],
        ),
      );
      final button = tester.widget<HouseButton>(
          find.byKey(const ValueKey('fx-console-snapshot')));
      expect(button.onPressed, isNull,
          reason: 'no composition, nothing to snapshot');
    });

    testWidgets('a flick in a direction runs that slice', (tester) async {
      final run = <String>[];
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'Scene',
          radial: [
            RadialEntry(label: 'Solid', run: () => run.add('solid')),
            RadialEntry(label: 'Text', run: () => run.add('text')),
            RadialEntry(label: 'Null', run: () => run.add('null')),
            RadialEntry(label: 'Camera', run: () => run.add('camera')),
          ],
          entries: [effect('Glow')],
        ),
      );

      // Flick straight up from the ring's centre, which is the first slice.
      final gesture = await tester.startGesture(tester.getCenter(centre()));
      await gesture.moveBy(const Offset(0, -(radialDeadZone + 30)));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();
      expect(run, ['solid'], reason: 'up is the first slice');
    });

    testWidgets('releasing inside the dead zone cancels', (tester) async {
      final run = <String>[];
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'Scene',
          radial: [
            RadialEntry(label: 'Solid', run: () => run.add('solid')),
            RadialEntry(label: 'Text', run: () => run.add('text')),
          ],
          entries: [effect('Glow')],
        ),
      );

      final gesture = await tester.startGesture(tester.getCenter(centre()));
      await gesture.moveBy(const Offset(0, -(radialDeadZone - 8)));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();
      expect(run, isEmpty,
          reason: 'opening and letting go without travelling picks nothing');
    });

    testWidgets('a slice with children expands in place, and the centre backs '
        'out', (tester) async {
      final run = <String>[];
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'My layer',
          radial: [
            RadialEntry(label: 'Duplicate', run: () => run.add('duplicate')),
            RadialEntry(label: 'New', children: [
              RadialEntry(label: 'Solid', run: () => run.add('solid')),
              RadialEntry(label: 'Text', run: () => run.add('text')),
            ]),
          ],
          entries: const [],
        ),
      );

      await tester.tap(find.byKey(const ValueKey('fx-radial-New')));
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('fx-radial-Solid')), findsOneWidget,
          reason: 'the ring expanded rather than running anything');
      expect(find.byKey(const ValueKey('fx-radial-Duplicate')), findsNothing);
      expect(find.text('New'), findsOneWidget,
          reason: 'the centre names the ring it is inside');
      expect(run, isEmpty);

      // The centre is the way back out.
      await tester.tap(centre());
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('fx-radial-Duplicate')), findsOneWidget);

      // Back in, and this time choose: the child runs and the console closes.
      await tester.tap(find.byKey(const ValueKey('fx-radial-New')));
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('fx-radial-Solid')));
      await tester.pumpAndSettle();
      expect(run, ['solid']);
      expect(query(), findsNothing, reason: 'a chosen child closes the console');
    });

    testWidgets('a flick expands a slice with children rather than closing',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'My layer',
          radial: [
            RadialEntry(label: 'Duplicate', run: () {}),
            RadialEntry(label: 'New', children: [
              RadialEntry(label: 'Solid', run: () {}),
            ]),
          ],
          entries: const [],
        ),
      );

      // Two slices: up is Duplicate, down is New. Flick down.
      final gesture = await tester.startGesture(tester.getCenter(centre()));
      await gesture.moveBy(const Offset(0, radialDeadZone + 30));
      await tester.pump();
      await gesture.up();
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('fx-radial-Solid')), findsOneWidget,
          reason: 'the console stays open, one ring deeper');
    });

    testWidgets('Escape pops a sub-ring before it closes the console',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'My layer',
          radial: [
            RadialEntry(label: 'New', children: [
              RadialEntry(label: 'Solid', run: () {}),
            ]),
          ],
          entries: const [],
        ),
      );
      await tester.tap(find.byKey(const ValueKey('fx-radial-New')));
      await tester.pumpAndSettle();

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(find.byKey(const ValueKey('fx-radial-New')), findsOneWidget,
          reason: 'one step back, not gone');

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pumpAndSettle();
      expect(query(), findsNothing);
    });

    testWidgets('a disabled slice keeps its place but does not run',
        (tester) async {
      final run = <String>[];
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'Nothing selected',
          radial: [
            RadialEntry(
                label: 'New composition', run: () => run.add('new')),
            RadialEntry(
                label: 'Import',
                enabled: false,
                run: () => run.add('import')),
          ],
          entries: const [],
        ),
      );

      expect(find.byKey(const ValueKey('fx-radial-Import')), findsOneWidget,
          reason: 'the ring keeps its shape, so directions stay learned');
      await tester.tap(find.byKey(const ValueKey('fx-radial-Import')));
      await tester.pumpAndSettle();
      expect(run, isEmpty);
    });

    testWidgets('with no radial entries the ring is not drawn at all',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'Scene',
          radial: const [],
          entries: [effect('Glow')],
        ),
      );
      expect(centre(), findsNothing,
          reason: 'an empty ring is hidden rather than drawn empty');
    });

    testWidgets('Enter on an empty bar closes rather than sitting inert',
        (tester) async {
      await open(
        tester,
        FxConsoleModel(
          radialTitle: 'Scene',
          radial: const [],
          entries: [effect('Glow')],
        ),
      );
      await tester.sendKeyEvent(LogicalKeyboardKey.enter);
      await tester.pumpAndSettle();
      expect(query(), findsNothing,
          reason: 'nothing chosen, nothing to run — done means done');
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
