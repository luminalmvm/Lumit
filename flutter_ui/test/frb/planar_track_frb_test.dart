// The Planar track effect's interface: its three buttons, its status line, and
// the span bar a partial track draws.
//
// Every document operation here is genuine; see frb_test_support.dart. What is
// *not* genuine is the track behind the status, and it cannot be: a planar
// track is the answer to an analysis of a real media file, and driving one is
// `lumit-render`'s own job (docs/impl/tracking.md §6). What the engine does with
// one — where the corners land, what the Corner pin gets keyed to — is asserted
// in Rust, in `crates/lumit-bridge/src/api/tests.rs`. What is asserted here is
// what this side does.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Planar track (frb)', () {
    /// A comp with one footage layer carrying an enabled Planar track,
    /// selected.
    ({LumitState state, LumitUiState uiState, LayerReference layer})
        withPlanarLayer() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final footage = p.state.project!.importFootage(path: 'C:/clips/sign.mov');
      comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'planar_track');
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;
      p.uiState.model.refresh();
      return (state: p.state, uiState: p.uiState, layer: layer);
    }

    testWidgets('four buttons, and a press reaches the engine',
        (tester) async {
      final p = withPlanarLayer();
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      final effect = p.layer.getEffects().single.id();
      expect(find.byKey(ValueKey<String>('fx-action-$effect-analyse')),
          findsOneWidget);
      expect(find.byKey(ValueKey<String>('fx-action-$effect-pin')),
          findsOneWidget,
          reason: 'the corner-pin gesture is a third Action row');
      expect(find.byKey(ValueKey<String>('fx-action-$effect-transform_keys')),
          findsOneWidget,
          reason: 'the transform gesture is the fourth');
      final cancel = find.byKey(ValueKey<String>('fx-action-$effect-cancel'));
      expect(cancel, findsOneWidget);

      // The quad's four corners are the effect's own rows — each an x/y pair,
      // folded into one point row the way every other pair is — and the
      // layer the pin lands on is a row beside them.
      expect(find.text('Upper left'), findsOneWidget);
      expect(find.text('Lower right'), findsOneWidget);
      expect(find.text('Pin layer'), findsOneWidget);
      // What the analysis is asked to follow.
      expect(find.text('Follow'), findsOneWidget);

      expect(
          find.byKey(const ValueKey('fx-planar-track-status')), findsOneWidget);
      expect(find.text('Not analysed yet'), findsOneWidget);

      // Cancel is accepted with nothing running and the engine records it,
      // which is the wiring proved end to end: without it the line could not
      // change, since a press moves nothing in the document.
      final before = p.state.project!.isDirty();
      await tester.tap(cancel);
      await tester.pump();
      expect(find.text('Analysis stopped'), findsOneWidget);
      expect(p.state.project!.isDirty(), before,
          reason: 'a press is an event, not an edit');
    });
  });
}
