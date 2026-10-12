// The Flow group on frb: the layer option, specified then built.
//
// Two things are being pinned here. That flow is reachable *only* as a switch —
// it left the in-between-frames dropdown, so it can no longer be picked as if
// it were a peer of Nearest and Blend — and that every parameter behind it
// actually reaches the document, which is what the whole group exists for after
// two decisions' worth of engine sat with no control surface at all.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/flow_rows_frb.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Flow group (frb)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    LayerReference footageLayer(dynamic p) {
      final footage =
          (p.state as LumitState).project!.importFootage(path: 'C:/c/shot.mov');
      (p.comp as CompositionReference)
          .addFootageLayer(footage: footage, asSequence: false);
      final layer = (p.comp as CompositionReference).getLayers().single;
      (p.uiState as LumitUiState).selectedLayer.value = layer;
      return layer;
    }

    Future<void> mount(WidgetTester tester, dynamic p) async {
      (p.uiState as LumitUiState)
          .workspace
          .interface
          .transformInEffectControls = true;
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: const Size(560, 900),
      ));
      await tester.pump();
    }

    testWidgets('the group is in the Timeline fold-out, where Transform lives',
        (tester) async {
      // The regression this pins: the group was first built into the Effect
      // controls panel, which hides its layer sections behind a setting that
      // is *off* by default — so turning flow on showed no controls at all.
      // The decision says "in the expanded layer", and the expanded layer is
      // the Timeline's twirl-down, which is where Transform actually is.
      final p = withComp();
      final layer = footageLayer(p);
      layer.setFlowEnabled(on_: true);
      expect(
        p.uiState.workspace.interface.transformInEffectControls,
        isFalse,
        reason: 'the default this was hidden behind',
      );

      final rows = layerFoldRows(
        entry: p.comp.getModel().layers.single,
        open: {flowPath(layer.internallayerId.toString())},
        hasAudio: false,
      );
      expect(
        rows.whereType<FoldGroupRow>().map((g) => g.label),
        contains('Flow'),
      );
      expect(
        rows.whereType<FoldFlowRow>().map((r) => r.kind).toSet(),
        FlowRowKind.values.toSet(),
        reason: 'every parameter has a row',
      );

      // And it is gone again when flow is off.
      layer.setFlowEnabled(on_: false);
      final without = layerFoldRows(
        entry: p.comp.getModel().layers.single,
        open: const {},
        hasAudio: false,
      );
      expect(without.whereType<FoldFlowRow>(), isEmpty);
      expect(
        without.whereType<FoldGroupRow>().map((g) => g.label),
        isNot(contains('Flow')),
      );
    });

    testWidgets('the engine is a row in the group, with two engines on it',
        (tester) async {
      final p = withComp();
      final layer = footageLayer(p);
      layer.setFlowEnabled(on_: true);
      await mount(tester, p);

      // Two, in the order the engine stores them: the code is the index, so a
      // list drawn the other way round would rename what stored projects say.
      expect(flowEngineOptions, ['Built in', 'RIFE']);

      await tester.tap(find.byKey(const ValueKey('flow-engine')));
      await tester.pumpAndSettle();
      expect(find.text('Built in'), findsWidgets);
      await tester.tap(find.text('RIFE').last);
      await tester.pumpAndSettle();

      // Written whole, as every row in this group is: one undo step, and the
      // seven settings beside it come back untouched.
      final after = layer.getFlowParams();
      expect(after.engine, 1, reason: 'the chosen engine reaches the document');
      expect(after.resolution, 0);
      expect(after.detail, 1);
      expect(after.smoothness, 50);
      expect(after.hudGuard, isTrue);
      expect(after.always, isFalse);
    });

    testWidgets('a cadence preset writes the rate it names', (tester) async {
      final p = withComp();
      final layer = footageLayer(p);
      layer.setFlowEnabled(on_: true);
      await mount(tester, p);

      // "On 2s" is 12 fps on 24 fps footage — the arithmetic an editor should
      // not have to do at the point of use.
      await tester.tap(find.byKey(const ValueKey('flow-input-rate-preset')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('On 2s (12)').last);
      await tester.pumpAndSettle();

      final rate = layer.getFlowInputRate();
      expect(rate, isA<BridgeScalar_Static>());
      expect((rate as BridgeScalar_Static).field0, 12.0);
    });

  }, skip: !engineAvailable);
}
