// Sync and Remove across the seam (docs/impl/custom-shader.md CS2), against
// the real engine: a shader edit offers rows and adopts none, Sync adopts
// them in one step, a keyframe on an adopted row is an ordinary keyframe, and
// undo walks back through each of those in turn.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart';
import 'package:lumit_flutter/panels/shader_editor.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb_test_support.dart';

/// A shader that compiles and declares one row of its own.
const String _oneRow = r"""
struct Params {
    /// @slider(0, 200) @default(25) @unit(px) Ripple radius
    radius: f32,
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    return lumit_sample(uv) * p.radius;
}
""";

/// The same shader with the row renamed: `radius` is no longer used.
const String _renamed = r"""
struct Params {
    /// @slider(0, 200) @default(25) @unit(px) Ripple reach
    reach: f32,
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    return lumit_sample(uv) * p.reach;
}
""";

void main() {
  setUpAll(initEngineForTests);

  group('Shader parameters (frb)', () {
    ({LumitState state, LumitUiState uiState, LayerReference layer})
        withShader() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'custom_shader');
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;
      return (state: p.state, uiState: p.uiState, layer: layer);
    }

    testWidgets('edit, derive, sync, keyframe, undo', (tester) async {
      final p = withShader();
      p.uiState.workspace.interface.transformInEffectControls = false;
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      final id = p.layer.getEffects().single.id();
      Future<void> refresh() async {
        p.uiState.model.refresh();
        await tester.pumpAndSettle();
      }

      BridgeParamSync sync() => p.layer.getEffects().single.parameterSync();
      BridgeEffectValue valueOf(String param) => p.layer
          .getInfo()
          .effects
          .single
          .values
          .firstWhere((e) => e.id == param)
          .value;

      // Edit: the editor's own commit writes the text and nothing else.
      expect(applyShaderSource(layer: p.layer, effect: id, source: _oneRow),
          isTrue);
      await refresh();

      // Derive: the row is drawn off the read model, flagged as the
      // instance's own, and the document does not hold it yet.
      expect(find.text('Ripple radius'), findsOneWidget);
      final derived = p.layer.getInfo().effects.single.derivedParams;
      expect(derived.map((r) => r.id), ['radius']);
      expect(derived.single.derived, isTrue);
      expect(cachedListParameters('custom_shader').any((r) => r.derived),
          isFalse, reason: 'the declared half never carries the flag');
      expect(sync().adds, ['radius'], reason: 'offered, not adopted');
      expect(sync().removes, isEmpty);

      // Sync: the adoption is one commit of its own.
      final adopting = p.layer.getEffects();
      expect(adopting.single.syncParameters(), ['radius']);
      p.layer.setEffects(effects: adopting);
      await refresh();
      expect(sync().adds, isEmpty, reason: 'the document holds it now');
      expect(find.text('Ripple radius'), findsOneWidget);

      // Keyframe: an adopted row keys through the same staged road as a
      // declared one.
      BridgeKeyframe key(int seconds, double value) => BridgeKeyframe(
            time: BridgeRational(num: seconds, den: 1),
            value: value,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          );
      final keying = p.layer.getEffects();
      keying.single.setValue(
          id: 'radius',
          value: BridgeEffectValue.float(
              BridgeScalar.keyframed([key(0, 10), key(2, 90)])));
      p.layer.setEffects(effects: keying);
      await refresh();
      expect(
          ((valueOf('radius') as BridgeEffectValue_Float).field0
                  as BridgeScalar_Keyframed)
              .field0
              .length,
          2);

      // A rename in the source leaves the keyed row where it is, says what
      // removing it would cost, and offers the new name.
      expect(applyShaderSource(layer: p.layer, effect: id, source: _renamed),
          isTrue);
      await refresh();
      expect(sync().adds, ['reach']);
      expect(sync().removes.map((r) => r.id), ['radius']);
      expect(sync().removes.single.keyframed, isTrue,
          reason: 'the removal would take two keys with it');
      expect(sync().removes.single.expression, isFalse);
      expect(find.text('Ripple radius'), findsNothing,
          reason: 'the source no longer draws it');
      expect(find.text('Ripple reach'), findsOneWidget);

      // Undo walks back one step at a time: the rename, the keys, the
      // adoption, the text.
      p.state.project!.undo();
      await refresh();
      expect(sync().adds, isEmpty);
      expect(sync().removes, isEmpty);
      expect(find.text('Ripple radius'), findsOneWidget);

      p.state.project!.undo();
      await refresh();
      expect((valueOf('radius') as BridgeEffectValue_Float).field0,
          isA<BridgeScalar_Static>(),
          reason: 'the keys were one step');

      p.state.project!.undo();
      await refresh();
      expect(sync().adds, ['radius'], reason: 'the adoption was one step');

      p.state.project!.undo();
      await refresh();
      expect(find.text('Ripple radius'), findsNothing,
          reason: 'and the text was the step before it');
    });

    // Without the built library there is nothing to test against; the harness
    // throws with the command to run.
  }, skip: !engineAvailable);
}
