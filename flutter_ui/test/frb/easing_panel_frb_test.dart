// The Easing panel: the editor with somewhere to live.
//
// What the panel is *for* is that it outlasts a selection change, so these ask
// the two questions a popup could not be asked — does the drawn shape survive
// the claim moving under it, and does Apply say so when there is nowhere to
// send one — plus the one that matters most: the shape it sends is the shape on
// screen.

import 'dart:io';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/easing_curve.dart';
import 'package:lumit_flutter/panels/easing_panel_frb.dart';
import 'package:lumit_flutter/panels/graph_maths.dart' show KeyEase;
import 'package:lumit_flutter/panels/key_ease_fields.dart';
import 'package:lumit_flutter/state/custom_easings.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/widgets/controls.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  Future<({LumitUiState ui, List<EasingCurve> applied})> mount(
    WidgetTester tester, {
    required bool claimed,
    double width = 320,
  }) async {
    final p = freshProject();
    final applied = <EasingCurve>[];
    if (claimed) p.uiState.easingApply.value = applied.add;
    // The panel lays out to the window it is given, not to the MediaQuery
    // size, and the default test window is a third of 800 across.
    tester.view.physicalSize = Size(width, 600);
    tester.view.devicePixelRatio = 1.0;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(hostPanel(
      child: const EasingPanelFrb(),
      state: p.state,
      uiState: p.uiState,
      size: Size(width, 600),
    ));
    await tester.pump();
    return (ui: p.uiState, applied: applied);
  }

  group('Easing panel (frb)', () {
    testWidgets('a preset tile applies its curve in one click', (tester) async {
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

    testWidgets('a custom curve applies what its handles show', (tester) async {
      final m = await mount(tester, claimed: true);

      // Take hold of the first handle — the easy ease it opens on puts it at
      // (1/3, 0) of the box - and drag it up and left. The box is sized to
      // the panel (docs/07 §5.4), so its side and margins are read off the
      // drawing: 20 across up to the box's largest, centred past that, and the
      // rest of the height split above and below.
      final paint = find.byKey(const ValueKey('easing-box'));
      final box = tester.getTopLeft(paint);
      final size = tester.getSize(paint);
      final side = (size.width - 40).clamp(170.0, 240.0);
      final boxLeft = (size.width - side) / 2;
      final marginY = (size.height - side) / 2;
      final gesture = await tester
          .startGesture(box + Offset(boxLeft + side / 3, marginY + side));
      await tester.pump();
      await gesture.moveBy(const Offset(-30, -60));
      await tester.pump();
      await gesture.up();
      await tester.pump();

      await tester.ensureVisible(find.byKey(const ValueKey('easing-apply')));
      await tester.tap(find.byKey(const ValueKey('easing-apply')));
      await tester.pump();

      // What went out is exactly what the handles show: the first control
      // point moved by (−30, −60) px of the box, the second untouched.
      final sent = m.applied.single;
      expect(sent.x1, closeTo(1 / 3 - 30 / side, 1e-6));
      expect(sent.y1, closeTo(60 / side, 1e-6));
      expect(sent.x2, 2 / 3);
      expect(sent.y2, 1);
    });

    // The selected key's own numbers, under the editor (docs/07 §5.4).
    testWidgets('one selected key puts its speed and influence under the box',
        (tester) async {
      final m = await mount(tester, claimed: true);
      expect(find.byKey(const ValueKey('easing-key')), findsNothing,
          reason: 'nothing at rest');

      final written = <KeyEase>[];
      m.ui.easingKey.value = KeyEaseClaim(
        channelId: 'c',
        index: 1,
        frame: 12,
        unit: 'px',
        ease: const KeyEase(
            inSpeed: 10, inInfluence: 0.25, outSpeed: 40, outInfluence: 0.5),
        apply: (_, __, edit) => written.add(edit),
      );
      await tester.pump();
      final section = find.byKey(const ValueKey('easing-key'));
      expect(section, findsOneWidget);
      expect(find.descendant(of: section, matching: find.text('f12')),
          findsOneWidget);
      for (final well in const [
        'speed-in',
        'influence-in',
        'speed-out',
        'influence-out'
      ]) {
        expect(find.byKey(ValueKey<String>('easing-key-$well')), findsOneWidget,
            reason: 'the $well well');
      }

      // A typed speed writes that side and nothing else: the two speeds
      // differed, so the Continuous tick opened unticked.
      tester
          .widget<DragValueField>(
              find.byKey(const ValueKey('easing-key-speed-out')))
          .onChanged(80);
      await tester.pump();
      expect(written, [const KeyEase(outSpeed: 80)]);

      // Ticking Continuous gives the out side the in side's speed; a speed
      // typed after that lands on both.
      tester
          .widget<HouseCheckbox>(
              find.byKey(const ValueKey('easing-key-continuous')))
          .onChanged!(true);
      await tester.pump();
      expect(written.last, const KeyEase(outSpeed: 10));
      tester
          .widget<DragValueField>(
              find.byKey(const ValueKey('easing-key-speed-in')))
          .onChanged(25);
      await tester.pump();
      expect(written.last, const KeyEase(inSpeed: 25, outSpeed: 25));

      // An influence is its own number, as a fraction.
      tester
          .widget<DragValueField>(
              find.byKey(const ValueKey('easing-key-influence-in')))
          .onChanged(75);
      await tester.pump();
      expect(written.last, const KeyEase(inInfluence: 0.75));

      // The claim withdrawn - several keys, or none - takes the section away.
      m.ui.easingKey.value = null;
      await tester.pump();
      expect(section, findsNothing);
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

    testWidgets(
        'a saved shape joins the grid, applies, and outlives the store '
        'being read again', (tester) async {
      final m = await mount(tester, claimed: true);
      await saveAs(tester, 'Snap', 'My ease');

      expect(find.text('My ease'), findsOneWidget);

      await tester.ensureVisible(find.text('My ease'));
      await tester.tap(find.text('My ease'));
      await tester.pump();
      expect(
          m.applied.last, easingPresets.firstWhere((p) => p.id == 'snap').curve,
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

  }, skip: !engineAvailable);
}
