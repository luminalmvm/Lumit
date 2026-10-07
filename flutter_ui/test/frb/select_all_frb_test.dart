// `Ctrl+A` is the focused panel's, not the composition's.
//
// `edit.select.all` used to mean "every layer" wherever it was pressed, so in
// the Project panel it selected things that were not on screen. The shell now
// routes the chord to whichever panel is focused — the same arrangement
// `Ctrl+F` uses for the search boxes — and falls back to every layer only when
// no panel claims it.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/state/dock.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Select all (frb)', () {
    /// The routing itself, with no panel mounted: which panels claim the chord
    /// and which leave it to mean "every layer".
    test('only the panels that keep a selection claim the chord', () {
      final p = freshProject();
      // The Node graph joined once its pick became a set: before that
      // a single-node selection had nothing to select *all* of, and claiming
      // the chord there would only have made it a dead key.
      for (final panel in [
        Panel.project,
        Panel.effectControls,
        Panel.graph,
      ]) {
        p.uiState.activePane.value = panel.pane();
        expect(p.uiState.requestSelectAll(), isTrue,
            reason: '${panel.name} answers Ctrl+A itself');
      }
      for (final panel in [Panel.timeline, Panel.viewer, null]) {
        p.uiState.activePane.value = panel?.pane();
        expect(p.uiState.requestSelectAll(), isFalse,
            reason: 'the shell still means every layer in ${panel?.name}');
      }
    });

    testWidgets('the Effect controls panel takes the whole stack',
        (tester) async {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'blur');
      layer.addEffect(name: 'vignette');
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;

      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      p.uiState.activePane.value = Panel.effectControls.pane();
      expect(p.uiState.requestSelectAll(), isTrue);
      await tester.pump();

      expect(p.uiState.selectedEffects.value, hasLength(2),
          reason: 'both effects on the layer, not just one');
      expect(p.uiState.selectedEffectsLayer, layer);
    });
  }, skip: !engineAvailable);
}

