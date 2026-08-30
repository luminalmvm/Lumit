// The reveal request (K-326's surviving half): asking the Timeline to show a
// property row opens the layer and exactly that row. The console's radial
// ring that once raised these asks went with the 2026-08-30 boards; the
// P/S/R/T/A reveal keys still make the same request, so the answer keeps its
// regression test.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  ({
    LumitState state,
    LumitUiState uiState,
    CompositionReference comp,
    LayerReference layer,
  }) withLayer() {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Scene');
    p.uiState.setSelectedComp(comp);
    final layer = comp.addSolidLayer();
    p.uiState.model.refresh();
    return (state: p.state, uiState: p.uiState, comp: comp, layer: layer);
  }

  group('the Timeline answers the reveal request (frb)', () {
    testWidgets('the keyed row is open in the fold-out after the ask',
        (tester) async {
      final p = withLayer();
      p.uiState.selectedLayer.value = p.layer;
      tester.view.physicalSize = const Size(1280, 600);
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const TimelinePanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1280, 600),
      ));
      await tester.pump();
      expect(find.text('Position'), findsNothing,
          reason: 'the layer starts folded shut');

      p.uiState
          .requestRevealProperty(p.layer.internallayerId, 'reveal.position');
      await tester.pump();
      expect(find.text('Position'), findsOneWidget,
          reason: 'the ask opens the layer and exactly that row');
      expect(p.uiState.revealPropertyRequest.value, isNull,
          reason: 'the request is consumed, not left to re-fire');

      // Asking again must never hide it — ensure-open, not the reveal keys'
      // toggle.
      p.uiState
          .requestRevealProperty(p.layer.internallayerId, 'reveal.position');
      await tester.pump();
      expect(find.text('Position'), findsOneWidget);
    });
  });
}
