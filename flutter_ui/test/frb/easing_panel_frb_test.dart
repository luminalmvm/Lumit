// The Easing panel: the editor with somewhere to live.
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
    testWidgets('a preset tile applies its curve in one click',
        (tester) async {
      final m = await mount(tester, claimed: true);

      await tester.ensureVisible(find.text('Back out'));
      await tester.tap(find.text('Back out'));
      await tester.pump();

      expect(m.applied, hasLength(1),
          reason: 'the tile itself applies — no confirming press');
      expect(m.applied.single,
          easingPresets.firstWhere((p) => p.id == 'backOut').curve);

      // And the tile loaded the box, so Apply sends the same shape again.
      await tester.ensureVisible(find.byKey(const ValueKey('easing-apply')));
      await tester.tap(find.byKey(const ValueKey('easing-apply')));
      await tester.pump();
      expect(m.applied, hasLength(2));
      expect(m.applied[1], m.applied[0]);
    });

    testWidgets('with nowhere to send a shape, a tile only loads the box',
        (tester) async {
      final m = await mount(tester, claimed: false);

      await tester.ensureVisible(find.text('Expo in'));
      await tester.tap(find.text('Expo in'));
      await tester.pump();
      expect(m.applied, isEmpty,
          reason: 'nothing was listening, so nothing was applied');

      // The claim arrives; the loaded shape is still in the box to send.
      m.ui.easingApply.value = m.applied.add;
      await tester.pump();
      await tester.ensureVisible(find.byKey(const ValueKey('easing-apply')));
      await tester.tap(find.byKey(const ValueKey('easing-apply')));
      await tester.pump();
      expect(m.applied.single,
          easingPresets.firstWhere((p) => p.id == 'expoIn').curve);
    });

    testWidgets(
        'the box stays up, so one shape goes to selection after '
        'selection', (tester) async {
      final m = await mount(tester, claimed: true);

      await tester.ensureVisible(find.text('Snap'));
      await tester.tap(find.text('Snap'));
      await tester.pump();
      await tester.ensureVisible(find.byKey(const ValueKey('easing-apply')));
      await tester.tap(find.byKey(const ValueKey('easing-apply')));
      await tester.pump();
      // The second press is the whole point of the panel: nothing was dismissed
      // in between, so the shape is still there to send again.
      await tester.tap(find.byKey(const ValueKey('easing-apply')));
      await tester.pump();

      expect(m.applied, hasLength(3),
          reason: 'the tile applied once and Apply twice more');
      expect(m.applied.toSet(), hasLength(1),
          reason: 'every press sent the same shape');
    });

    testWidgets('a drawn shape survives the claim going and coming back',
        (tester) async {
      final m = await mount(tester, claimed: true);
      await tester.ensureVisible(find.text('Sine in'));
      await tester.tap(find.text('Sine in'));
      await tester.pump();
      m.applied.clear();

      // The Timeline switches to the speed lens and back — the claim drops and
      // is republished. The editor's own State must not be rebuilt by it, or
      // the shape the user drew would silently reset to the default.
      m.ui.easingApply.value = null;
      await tester.pump();
      m.ui.easingApply.value = m.applied.add;
      await tester.pump();

      await tester.ensureVisible(find.byKey(const ValueKey('easing-apply')));
      await tester.tap(find.byKey(const ValueKey('easing-apply')));
      await tester.pump();
      expect(m.applied.single,
          easingPresets.firstWhere((p) => p.id == 'sineIn').curve,
          reason: 'the drawn shape outlived the claim');
    });

    testWidgets('a custom curve applies what its handles show', (tester) async {
      final m = await mount(tester, claimed: true);

      // Take hold of the first handle — the easy ease it opens on puts it at
      // (1/3, 0) of the 170 px box, whose margins are 20 across and 85 down —
      // and drag it up and left.
      final box = tester.getTopLeft(find.byKey(const ValueKey('easing-box')));
      final gesture =
          await tester.startGesture(box + const Offset(20 + 170 / 3, 85 + 170));
      await tester.pump();
      await gesture.moveBy(const Offset(-30, -60));
      await tester.pump();
      await gesture.up();
      await tester.pump();

      await tester.ensureVisible(find.byKey(const ValueKey('easing-apply')));
      await tester.tap(find.byKey(const ValueKey('easing-apply')));
      await tester.pump();

      // What went out is exactly what the handles show: the first control
      // point moved by (−30, −60) px of a 170 px box, the second untouched.
      final sent = m.applied.single;
      expect(sent.x1, closeTo(1 / 3 - 30 / 170, 1e-6));
      expect(sent.y1, closeTo(60 / 170, 1e-6));
      expect(sent.x2, 2 / 3);
      expect(sent.y2, 1);
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
      await tester.ensureVisible(find.text(preset));
      await tester.tap(find.text(preset));
      await tester.pump();
      await tester.ensureVisible(find.byKey(const ValueKey('easing-save')));
      await tester.tap(find.byKey(const ValueKey('easing-save')));
      await tester.pump();
      await tester.enterText(find.byKey(const ValueKey('easing-name')), name);
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pumpAndSettle();
    }

    testWidgets('a saved shape joins the grid, applies, and outlives the store '
        'being read again', (tester) async {
      final m = await mount(tester, claimed: true);
      await saveAs(tester, 'Snap', 'My ease');

      expect(find.text('My ease'), findsOneWidget);

      await tester.ensureVisible(find.text('My ease'));
      await tester.tap(find.text('My ease'));
      await tester.pump();
      expect(m.applied.last,
          easingPresets.firstWhere((p) => p.id == 'snap').curve,
          reason: 'a custom eases the selection exactly as a stock preset '
              'does, and its tile applies in one click too');

      // The whole point: it is kept on disk, not in the widget. Read the store
      // again from nothing and put a fresh panel up over it.
      CustomEasings.reload();
      expect(CustomEasings.all.single.name, 'My ease');
      expect(CustomEasings.all.single.curve,
          easingPresets.firstWhere((p) => p.id == 'snap').curve);
      await mount(tester, claimed: true);
      expect(find.text('My ease'), findsOneWidget,
          reason: 'a saved shape is still in the grid on the next launch');
    });

    testWidgets('the shipped shapes are untouched by one of the user\'s own',
        (tester) async {
      await mount(tester, claimed: true);
      await saveAs(tester, 'Heavy ease', 'Mine');

      for (final preset in easingPresets) {
        expect(find.text(easingPresetName(preset.id)), findsOneWidget,
            reason: 'every shipped shape is still in the grid');
      }
      // And there is nothing to delete on one of them: the menu is only on the
      // saved shapes.
      await tester.ensureVisible(find.text('Heavy ease'));
      await tester.tap(find.text('Heavy ease'), buttons: kSecondaryButton);
      await tester.pumpAndSettle();
      expect(find.text('Delete'), findsNothing);
    });

    testWidgets('rename keeps the shape, delete takes it away',
        (tester) async {
      await mount(tester, claimed: true);
      await saveAs(tester, 'Back out', 'First try');

      await tester.ensureVisible(find.text('First try'));
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
          easingPresets.firstWhere((p) => p.id == 'backOut').curve,
          reason: 'a rename changes the name and nothing else');

      await tester.ensureVisible(find.text('Bounce'));
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
