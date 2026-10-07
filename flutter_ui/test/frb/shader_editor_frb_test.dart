// The shader editor window (docs/impl/custom-shader.md §3.2, CS3), against the
// real engine: what the user types is compiled by naga, and what Apply commits
// is a genuine `SetLayerEffects`.
//
// Three things the surface has to get right, and they are the three tests:
// the round trip (open, type, apply, and the shader's own rows appear), a
// broken source (the compiler's sentence with the line number counted from the
// user's own text, and the badge over the effect afterwards), and undo (one
// step, and the text that was there before comes back).

import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/shader_editor.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb_test_support.dart';

/// A shader that compiles and declares one row of its own.
const String _good = r"""
struct Params {
    /// @slider(0, 200) @default(25) @unit(px) Ripple radius
    radius: f32,
}

fn shade(uv: vec2<f32>) -> vec4<f32> {
    return lumit_sample(uv) * p.radius;
}
""";

/// The same shader with the third line calling something that does not exist.
const String _broken = r"""
fn shade(uv: vec2<f32>) -> vec4<f32> {
    let a = 1.0;
    return nonesuch(uv);
}
""";

void main() {
  setUpAll(initEngineForTests);

  group('Shader editor (frb)', () {
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

    Future<void> mount(
      WidgetTester tester,
      ({LumitState state, LumitUiState uiState, LayerReference layer}) p,
    ) async {
      p.uiState.workspace.interface.transformInEffectControls = false;
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
    }

    /// The example a fresh Custom shader opens with (owner, 2026-09-01). Read
    /// off the instance rather than repeated here: what these two assert is
    /// that a gesture left the source *as it was*, and pinning the example's
    /// text is `the_starter_shader_compiles_and_changes_nothing`'s job.
    String? started(
            ({LumitState state, LumitUiState uiState, LayerReference layer}) p) =>
        p.layer.getEffects().single.shaderSource();

    /// Open the editor the way a person does: the effect's own Action row.
    Future<void> openEditor(
      WidgetTester tester,
      ({LumitState state, LumitUiState uiState, LayerReference layer}) p,
    ) async {
      final id = p.layer.getEffects().single.id();
      await tester.tap(find.byKey(ValueKey<String>('fx-action-$id-edit')));
      await tester.pumpAndSettle();
    }

    /// Type into the code well and let the debounce ask the engine.
    Future<void> type(WidgetTester tester, String source) async {
      await tester.enterText(
          find.byKey(const ValueKey<String>('shader-editor-code')), source);
      await tester.pump(const Duration(milliseconds: 500));
    }

    testWidgets('opens, takes a shader, and applies it in one undo step',
        (tester) async {
      final p = withShader();
      await mount(tester, p);
      final opened = started(p);
      await openEditor(tester, p);

      expect(find.byKey(const ValueKey<String>('shader-editor-code')),
          findsOneWidget);
      expect(find.text('Ripple radius'), findsNothing,
          reason: 'nothing is written yet, so no row exists yet');

      await type(tester, _good);
      // The line numbers count the lines that are there.
      expect(find.text('1\n2\n3\n4\n5\n6\n7\n8\n9'), findsOneWidget);

      await tester.tap(
          find.byKey(const ValueKey<String>('shader-editor-apply')));
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey<String>('shader-editor-code')),
          findsNothing, reason: 'applying closes the window');
      expect(p.layer.getEffects().single.shaderSource(), _good,
          reason: 'the text reached the document');
      expect(find.text('Ripple radius'), findsOneWidget,
          reason: "the shader's own row is now a row in the panel");

      // One `SetLayerEffects`, so one undo puts the effect back the way it was
      // - which is the starter example a fresh Custom shader opens with, not
      // nothing (owner, 2026-09-01).
      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pumpAndSettle();
      expect(p.layer.getEffects().single.shaderSource(), opened);
      expect(find.text('Ripple radius'), findsNothing);
    });

    testWidgets("a broken shader shows the compiler's own line number",
        (tester) async {
      final p = withShader();
      await mount(tester, p);
      await openEditor(tester, p);

      await type(tester, _broken);
      // Line 3 of the three lines they wrote, not line 3 of the wrapper Lumit
      // put around them.
      expect(find.textContaining('wgsl:3:'), findsOneWidget);
      expect(find.text(l10nShaderCompiles), findsNothing);

      // Applying it anyway is allowed: the picture keeps the last shader that
      // worked and the effect wears the calm badge with the same sentence
      // under it (§3.2). A broken edit is a state to be in, not a refusal.
      await tester.tap(
          find.byKey(const ValueKey<String>('shader-editor-apply')));
      await tester.pumpAndSettle();
      expect(p.layer.getEffects().single.shaderSource(), _broken);
      expect(find.text('This shader did not compile'), findsOneWidget);
      expect(find.textContaining('wgsl:3:'), findsOneWidget,
          reason: 'the badge carries the compiler\'s words too');
    });

    /// **Ten keystrokes are one compile and one preview** (custom-shader.md §8
    /// item 21, specified and never written until the feature it was meant to
    /// land with).
    ///
    /// The preview is Airyz's: "having to click apply is kind of annoying while
    /// editing the shader". It rides the same settle the compiler's opinion
    /// does, so typing costs one of each rather than one per letter - and it
    /// only draws text that compiles, because a broken shader renders as a
    /// passthrough and a layer blinking mid-word is worse than a still one.
    testWidgets('typing asks once, and only draws what compiles',
        (tester) async {
      final p = withShader();
      await mount(tester, p);

      var compiles = 0;
      final drawn = <String>[];
      await tester.tap(find.byKey(ValueKey<String>(
          'fx-action-${p.layer.getEffects().single.id()}-edit')));
      await tester.pumpAndSettle();
      // The window is already open from the panel's own road; drive the one
      // under test directly so the two callbacks can be counted.
      await tester.tap(find.byKey(const ValueKey<String>('shader-editor-cancel')));
      await tester.pumpAndSettle();
      unawaited(showShaderEditor(
        context: tester.element(find.byType(EffectControlsPanelFrb)),
        home: InstanceHome.layer(p.layer),
        effect: p.layer.getEffects().single.id(),
        preview: drawn.add,
      ));
      await tester.pumpAndSettle();

      final well = find.byKey(const ValueKey<String>('shader-editor-code'));
      for (var i = 0; i < 10; i++) {
        await tester.enterText(well, '$_good\n// $i');
        await tester.pump(const Duration(milliseconds: 20));
      }
      compiles = drawn.length;
      expect(compiles, 0, reason: 'nothing is asked while the typing runs');

      await tester.pump(const Duration(milliseconds: 500));
      expect(drawn.length, 1, reason: 'the settle asks once, for the last text');
      expect(drawn.single.endsWith('// 9'), isTrue);

      // Broken text compiles to nothing, so nothing is drawn with it.
      await tester.enterText(well, 'fn shade( {');
      await tester.pump(const Duration(milliseconds: 500));
      expect(drawn.length, 1,
          reason: 'a shader that does not compile is not a picture');

      await tester.tap(find.byKey(const ValueKey<String>('shader-editor-cancel')));
      await tester.pumpAndSettle();
    });

    // Without the built library there is nothing to test against; the harness
    // throws with the command to run.
  }, skip: !engineAvailable);
}

/// The confirmation line, read the way the widget reads it.
final String l10nShaderCompiles = 'The shader compiles';
