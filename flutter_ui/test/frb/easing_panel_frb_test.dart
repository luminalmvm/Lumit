// The Easing panel (K-349): the editor with somewhere to live.
//
// What the panel is *for* is that it outlasts a selection change, so these ask
// the two questions a popup could not be asked — does the drawn shape survive
// the claim moving under it, and does Apply say so when there is nowhere to
// send one — plus the one that matters most: the shape it sends is the shape on
// screen.

import 'dart:io';

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/easing_curve.dart';
import 'package:lumit_flutter/panels/easing_editor.dart';
import 'package:lumit_flutter/panels/easing_panel_frb.dart';
import 'package:lumit_flutter/state/custom_easings.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  Future<({LumitUiState ui, List<EasingCurve> applied})> mount(
    WidgetTester tester, {
    required bool claimed,
  }) async {
    final p = freshProject();
    final applied = <EasingCurve>[];
    if (claimed) p.uiState.easingApply.value = applied.add;
    await tester.pumpWidget(hostPanel(
      child: const EasingPanelFrb(),
      state: p.state,
      uiState: p.uiState,
      size: const Size(320, 600),
    ));
    await tester.pump();
    return (ui: p.uiState, applied: applied);
  }

  group('Easing panel (frb)', () {
    testWidgets('Apply sends the shape that is on screen', (tester) async {
      final m = await mount(tester, claimed: true);

      await tester.tap(find.text('Overshoot'));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('easing-apply')));
      await tester.pump();

      expect(m.applied, hasLength(1));
      expect(m.applied.single,
          easingPresets.firstWhere((p) => p.id == 'overshoot').curve);
    });

    testWidgets(
        'the box stays up, so one shape goes to selection after '
        'selection', (tester) async {
      final m = await mount(tester, claimed: true);

      await tester.tap(find.text('Snap'));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('easing-apply')));
      await tester.pump();
      // The second press is the whole point of the panel: nothing was dismissed
      // in between, so the shape is still there to send again.
      await tester.tap(find.byKey(const ValueKey('easing-apply')));
      await tester.pump();

      expect(m.applied, hasLength(2));
      expect(m.applied[0], m.applied[1]);
    });

    testWidgets('a drawn shape survives the claim going and coming back',
        (tester) async {
      final m = await mount(tester, claimed: true);
      await tester.tap(find.text('Anticipate'));
      await tester.pump();

      // The Timeline switches to the speed lens and back — the claim drops and
      // is republished. The editor's own State must not be rebuilt by it, or
      // the shape the user drew would silently reset to the default.
      m.ui.easingApply.value = null;
      await tester.pump();
      m.ui.easingApply.value = m.applied.add;
      await tester.pump();

      await tester.tap(find.byKey(const ValueKey('easing-apply')));
      await tester.pump();
      expect(m.applied.single,
          easingPresets.firstWhere((p) => p.id == 'anticipate').curve,
          reason: 'the drawn shape outlived the claim');
    });

    testWidgets('with nowhere to send a shape, Apply is dead and says why',
        (tester) async {
      await mount(tester, claimed: false);

      expect(find.text('Shapes apply in the value lens.'), findsOneWidget);
      final apply = tester
          .widget<HouseButton>(find.byKey(const ValueKey('easing-apply')));
      expect(apply.onPressed, isNull,
          reason: 'a persistent panel must not offer a button that does '
              'nothing');
    });

    testWidgets('the reason goes away once there is somewhere to send it',
        (tester) async {
      final m = await mount(tester, claimed: false);
      expect(find.text('Shapes apply in the value lens.'), findsOneWidget);

      m.ui.easingApply.value = m.applied.add;
      await tester.pump();

      expect(find.text('Shapes apply in the value lens.'), findsNothing);
      final apply = tester
          .widget<HouseButton>(find.byKey(const ValueKey('easing-apply')));
      expect(apply.onPressed, isNotNull);
    });

    testWidgets('there is no Close button — a panel is not dismissed',
        (tester) async {
      await mount(tester, claimed: true);
      expect(find.text('Close'), findsNothing);
      // Apply is the panel's filled action, and a filled action wears its
      // capitals as a style rather than as a second string (§7.1).
      expect(find.text('Apply'.toUpperCase()), findsOneWidget);
    });
  }, skip: !engineAvailable);

  // Shapes of the user's own (item R): saved beside the settings, shown in the
  // same row as the seven that ship, applied by the same road.
  group('Custom easings (frb)', () {
    setUp(() {
      // Its own scratch folder per test, so one test's collection is never
      // another's — the store is a file, and a file outlives a test.
      Workspace.storeOverride =
          '${Directory.systemTemp.createTempSync('lumit-eas').path}'
          '/workspace.json';
      CustomEasings.reload();
    });

    /// Draw [preset] into the box and keep it as [name].
    Future<void> saveAs(WidgetTester tester, String preset, String name) async {
      await tester.tap(find.text(preset));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('easing-save')));
      await tester.pump();
      await tester.enterText(find.byKey(const ValueKey('easing-name')), name);
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
    }

    testWidgets('a saved shape joins the row, applies, and outlives the store '
        'being read again', (tester) async {
      final m = await mount(tester, claimed: true);
      await saveAs(tester, 'Snap', 'My ease');

      expect(find.text('My ease'), findsOneWidget);

      await tester.tap(find.text('My ease'));
      await tester.pump();
      await tester.tap(find.byKey(const ValueKey('easing-apply')));
      await tester.pump();
      expect(m.applied.single,
          easingPresets.firstWhere((p) => p.id == 'snap').curve,
          reason: 'a custom eases the selection exactly as a stock preset does');

      // The whole point: it is kept on disk, not in the widget. Read the store
      // again from nothing and put a fresh panel up over it.
      CustomEasings.reload();
      expect(CustomEasings.all.single.name, 'My ease');
      expect(CustomEasings.all.single.curve,
          easingPresets.firstWhere((p) => p.id == 'snap').curve);
      await mount(tester, claimed: true);
      expect(find.text('My ease'), findsOneWidget,
          reason: 'a saved shape is still in the row on the next launch');
    });

    testWidgets('the shipped shapes are untouched by one of the user\'s own',
        (tester) async {
      await mount(tester, claimed: true);
      await saveAs(tester, 'Heavy ease', 'Mine');

      for (final preset in easingPresets) {
        expect(find.text(easingPresetName(preset.id)), findsOneWidget,
            reason: 'every shipped shape is still in the row');
      }
      // And there is nothing to delete on one of them: the menu is only on the
      // saved shapes.
      await tester.tap(find.text('Heavy ease'), buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      expect(find.text('Delete'), findsNothing);
    });

    testWidgets('rename keeps the shape, delete takes it away',
        (tester) async {
      await mount(tester, claimed: true);
      await saveAs(tester, 'Overshoot', 'First try');

      await tester.tap(find.text('First try'), buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Rename'));
      await tester.pumpAndSettle();
      await tester.enterText(
          find.byKey(const ValueKey('easing-name')), 'Bounce');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();

      expect(find.text('First try'), findsNothing);
      expect(find.text('Bounce'), findsOneWidget);
      expect(CustomEasings.all.single.curve,
          easingPresets.firstWhere((p) => p.id == 'overshoot').curve,
          reason: 'a rename changes the name and nothing else');

      await tester.tap(find.text('Bounce'), buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      await tester.tap(find.text('Delete'));
      await tester.pumpAndSettle();

      expect(find.text('Bounce'), findsNothing);
      expect(CustomEasings.all, isEmpty);
      CustomEasings.reload();
      expect(CustomEasings.all, isEmpty, reason: 'and it stays deleted');
    });
  }, skip: !engineAvailable);
}
