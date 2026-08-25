// The Easing panel (K-349): the editor with somewhere to live.
//
// What the panel is *for* is that it outlasts a selection change, so these ask
// the two questions a popup could not be asked — does the drawn shape survive
// the claim moving under it, and does Apply say so when there is nowhere to
// send one — plus the one that matters most: the shape it sends is the shape on
// screen.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/easing_curve.dart';
import 'package:lumit_flutter/panels/easing_panel_frb.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Easing panel (frb)', () {
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
}
