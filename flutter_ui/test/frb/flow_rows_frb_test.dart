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
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/retime.dart';

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

    testWidgets('the group appears only while flow is on', (tester) async {
      final p = withComp();
      final layer = footageLayer(p);
      await mount(tester, p);

      // The section heading is a kicker, so its word reaches the screen
      // capitalised while the string itself stays sentence case.
      expect(find.text('FLOW'), findsNothing,
          reason: 'a layer not using flow has no flow group');

      layer.setFlowEnabled(on_: true);
      await mount(tester, p);
      expect(find.text('FLOW'), findsOneWidget);
      expect(layer.getInterpolation(), BridgeRetimeInterp.flow);
    });

    testWidgets('flow is a switch, not a dropdown entry', (tester) async {
      final p = withComp();
      final layer = footageLayer(p);
      await mount(tester, p);

      // The dropdown is still there for Nearest/Blend...
      expect(find.byKey(const ValueKey('src-retime-interp')), findsOneWidget);
      // ...but flow is no longer one of the things it offers. Picking it there
      // made the most expensive setting a layer has look like a small one.
      expect(find.text('Optical flow'), findsNothing);

      // The switch is what turns it on, and it round-trips.
      expect(layer.getFlowEnabled(), isFalse);
      layer.setFlowEnabled(on_: true);
      expect(layer.getFlowEnabled(), isTrue);
      layer.setFlowEnabled(on_: false);
      expect(layer.getFlowEnabled(), isFalse);
      expect(layer.getInterpolation(), BridgeRetimeInterp.nearest,
          reason: 'turning flow off returns the layer to the crisp default');
    });

    testWidgets('every parameter reaches the document', (tester) async {
      final p = withComp();
      final layer = footageLayer(p);
      layer.setFlowEnabled(on_: true);
      await mount(tester, p);

      // Sections start twirled open, so the rows are already built.
      for (final key in [
        'flow-resolution',
        'flow-detail',
        'flow-smoothness',
        'flow-occlusion',
        'flow-fallback',
        'flow-hud-guard',
        'flow-always',
      ]) {
        expect(find.byKey(ValueKey(key)), findsOneWidget,
            reason: '$key is one of the parameters docs/08 §3.1 specifies');
      }

      // Defaults, straight from the engine.
      final before = layer.getFlowParams();
      expect(before.resolution, 0, reason: 'native');
      expect(before.detail, 1, reason: 'medium');
      expect(before.smoothness, 50);
      expect(before.hudGuard, isTrue,
          reason: 'game capture is the primary footage, so the guard is on');
      expect(before.always, isFalse);

      // A write of the whole group round-trips.
      layer.setFlowParams(
        params: BridgeFlowParams(
          resolution: 2,
          detail: 3,
          smoothness: 12.5,
          occlusion: 1,
          fallback: 1,
          hudGuard: false,
          always: true,
        ),
      );
      final after = layer.getFlowParams();
      expect(after.resolution, 2);
      expect(after.detail, 3);
      expect(after.smoothness, 12.5);
      expect(after.occlusion, 1);
      expect(after.fallback, 1);
      expect(after.hudGuard, isFalse);
      expect(after.always, isTrue);
    });

    testWidgets('the input rate has a control, defaulting to Auto',
        (tester) async {
      final p = withComp();
      final layer = footageLayer(p);
      layer.setFlowEnabled(on_: true);
      await mount(tester, p);

      expect(find.byKey(const ValueKey('flow-input-rate')), findsOneWidget);
      expect(
          find.byKey(const ValueKey('flow-input-rate-preset')), findsOneWidget);
      // Auto is 0 — adjacent source frames, the clip's own rate.
      final auto = layer.getFlowInputRate();
      expect(auto, isA<BridgeScalar_Static>());
      expect((auto as BridgeScalar_Static).field0, lessThan(0.5));
      expect(find.text('Auto'), findsOneWidget);
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

    testWidgets('the input rate is keyframeable, so a cadence can change',
        (tester) async {
      final p = withComp();
      final layer = footageLayer(p);
      layer.setFlowEnabled(on_: true);
      await mount(tester, p);

      // Anime commonly switches between 2s and 3s inside one cut, so the
      // conform has to be able to follow it rather than being one number for
      // the whole clip (the reason for a value field over a preset list).
      expect(find.byKey(const ValueKey('kf-stopwatch-flow-input-rate')),
          findsOneWidget);
      await tester
          .tap(find.byKey(const ValueKey('kf-stopwatch-flow-input-rate')));
      await tester.pumpAndSettle();
      expect(layer.getFlowInputRate(), isA<BridgeScalar_Keyframed>(),
          reason: 'the stopwatch plants a key and the rate becomes a curve');
    });

    testWidgets('switching flow off parks the group and back on restores it',
        (tester) async {
      final p = withComp();
      final layer = footageLayer(p);
      layer.setFlowEnabled(on_: true);
      layer.setFlowParams(
        params: BridgeFlowParams(
          resolution: 1,
          detail: 3,
          smoothness: 80,
          occlusion: 1,
          fallback: 1,
          hudGuard: false,
          always: false,
        ),
      );
      layer.setFlowEnabled(on_: false);
      // Comparing a flow shot against the plain one is a normal thing to do and
      // must not cost the tuning that got you there: while the policy is
      // Nearest the group waits in the layer's parked_flow, and the panel keeps
      // showing what it would come back to.
      expect(layer.getFlowParams().detail, 3);

      layer.setFlowEnabled(on_: true);
      expect(layer.getFlowParams().resolution, 1);
      expect(layer.getFlowParams().detail, 3);
      expect(layer.getFlowParams().smoothness, 80);
    });
  }, skip: !engineAvailable);
}
