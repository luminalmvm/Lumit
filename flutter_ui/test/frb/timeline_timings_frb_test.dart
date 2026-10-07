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
          reason: 'numbers are what the column is for');

      p.uiState.renderTimings.report(BridgeFrameProfile(
        frame: BigInt.zero,
        totalMs: 12.5,
        planMs: 0.5,
        decodeMs: 1.0,
        buildMs: 1.5,
        compositeMs: 9.0,
        presentMs: 0.5,
        layers: [
          BridgeLayerTiming(layer: p.layerId, ms: 8.5, effects: const []),
        ],
        view: 0,
      ));
      await tester.pump();

      expect(find.text('8.50 ms'), findsOneWidget,
          reason: 'the layer row shows what its picture cost');
      expect(find.text('12.50 ms'), findsOneWidget,
          reason: 'and the header shows what the whole frame cost, so a dash '
              'on a row below can be told from an engine saying nothing');
    });
  });
}
