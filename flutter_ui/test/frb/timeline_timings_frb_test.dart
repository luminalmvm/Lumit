// The render-time column as a user meets it: the stopwatch in its header has to
// be reachable, and the numbers have to land on the rows.
//
// **Why this is a test and not an assumption.** The header cell lives inside the
// column-group `Draggable`/`DragTarget` that reorders the outline's clusters, so
// "does a tap on it reach the switch?" is a real question with a real way to be
// wrong — and if the answer were no, the column would look exactly like a
// feature that does not work: a header, a row per layer, and nothing in them
// ever (which is how it was reported).

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/panels/timeline_timings.dart';
import 'package:lumit_flutter/shell/status_line_frb.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('The Timeline render-time column (frb)', () {
    ({LumitState state, LumitUiState uiState, String layerId}) withLayer() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      p.uiState.setSelectedComp(comp);
      return (
        state: p.state,
        uiState: p.uiState,
        layerId: comp.getLayers().single.internallayerId.toString(),
      );
    }

    /// The same, with one effect on the layer — for the row that carries an
    /// effect's own cost.
    ({LumitState state, LumitUiState uiState, String layerId, String effectId})
        withEffect() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'blur');
      p.uiState.setSelectedComp(comp);
      return (
        state: p.state,
        uiState: p.uiState,
        layerId: layer.internallayerId.toString(),
        effectId: layer.getEffects().single.id().toString(),
      );
    }

    Future<void> mount(WidgetTester tester, dynamic p) async {
      // A window wide enough to hold the whole outline: the render-time column
      // is its rightmost, and the test is about reaching it rather than about
      // what a narrow window hides.
      tester.view.physicalSize = const Size(1600, 700);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(1600, 700),
        child: const TimelinePanelFrb(),
      ));
      await tester.pump();
      await settleFrb(tester, minRounds: 6);
    }

    testWidgets('the column measures by default and shows the frame total',
        (tester) async {
      final p = withLayer();
      await mount(tester, p);

      expect(p.uiState.renderTimings.measuring, isTrue,
          reason: 'numbers are what the column is for (K-276 revision)');

      p.uiState.renderTimings.report(BridgeFrameProfile(
        frame: BigInt.zero,
        totalMs: 12.5,
        layers: [
          BridgeLayerTiming(layer: p.layerId, ms: 8.5, effects: const []),
        ],
      ));
      await tester.pump();

      expect(find.text('8.50 ms'), findsOneWidget,
          reason: 'the layer row shows what its picture cost');
      expect(find.text('12.50 ms'), findsOneWidget,
          reason: 'and the header shows what the whole frame cost, so a dash '
              'on a row below can be told from an engine saying nothing');
    });

    /// The switch is in the bottom strip now, not in the column header: a
    /// header that says Time over a column of dashes gives no hint that it is
    /// a button, which is exactly how the feature was reported broken.
    testWidgets('the header is a readout, and the strip carries the switch',
        (tester) async {
      final p = withLayer();
      tester.view.physicalSize = const Size(1600, 700);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(1600, 700),
        child: const Column(children: [
          Expanded(child: TimelinePanelFrb()),
          StatusLineFrb(),
        ]),
      ));
      await tester.pump();
      await settleFrb(tester, minRounds: 6);

      // Clicking the header changes nothing — it is a readout now, and there
      // is not even a gesture detector under it to claim the tap.
      await tester.tapAt(
          tester.getTopLeft(find.byType(TimingsHeaderCell)) +
              const Offset(4, 8));
      await tester.pump();
      expect(p.uiState.renderTimings.measuring, isTrue);

      // The strip's clock stops it, and stopping takes the whole column with
      // it — stale numbers must not sit on screen looking current.
      await tester.tap(find.byType(RenderTimingsToggle));
      await tester.pump();
      expect(p.uiState.renderTimings.measuring, isFalse);
      await tester.pump();
      expect(find.byType(TimingsHeaderCell), findsNothing,
          reason: 'switching measuring off takes the column with it');

      await tester.tap(find.byType(RenderTimingsToggle));
      await tester.pump();
      expect(p.uiState.renderTimings.measuring, isTrue);
      await tester.pump();
      expect(find.byType(TimingsHeaderCell), findsOneWidget,
          reason: 'switching measuring back on brings the column back');
      // Wait for the render itself to come back, not for a fixed number of
      // rounds: a frame's wall-clock cost varies with the machine, and a slow
      // one leaves the progress tracker's timer pending past the end of the
      // test.
      await settleFrb(
        tester,
        until: () => p.uiState.previewProgress.idle,
        maxRounds: 100,
      );
    });

    /// An effect's own cost belongs in the same column as its layer's, or the
    /// two cannot be read against each other at a glance.
    testWidgets('an effect heading puts its number in the layer column',
        (tester) async {
      final p = withEffect();
      await mount(tester, p);

      // Twirl the layer open, then its Effects group, so the effect heading is
      // on screen. Near the left end, not the centre: a fold row spans the
      // whole outline, which is wider than a click can assume.
      await tester.tap(find.byKey(ValueKey<String>('tl-twirl-${p.layerId}')));
      await tester.pump();
      await tester.tapAt(tester.getTopLeft(find.byKey(
              ValueKey<String>('tl-group-${p.layerId}/effects'))) +
          const Offset(5, 8));
      await tester.pumpAndSettle();

      p.uiState.renderTimings.report(BridgeFrameProfile(
        frame: BigInt.zero,
        totalMs: 20,
        layers: [
          BridgeLayerTiming(
            layer: p.layerId,
            ms: 8.5,
            effects: [BridgeEffectTiming(effect: p.effectId, ms: 4.5)],
          ),
        ],
      ));
      await tester.pump();

      final layerNumber = tester.getRect(find.text('8.50 ms'));
      final effectNumber = tester.getRect(find.text('4.50 ms'));
      expect(effectNumber.right, closeTo(layerNumber.right, 0.5),
          reason: 'the two numbers share a column, so they read as one');
    });
  });
}
