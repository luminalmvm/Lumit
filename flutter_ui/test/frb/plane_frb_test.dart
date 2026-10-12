// The planes tier's interface: the two buttons Depth and Remove background
// share, their status line, the span bar under it, and the badge they wear on a
// machine with no model (docs/08 §3.104 and §3.105, docs/impl/addons.md §6.1).
//
// Every document operation here is genuine; see frb_test_support.dart. What is
// *not* genuine is a read shot, and it cannot be: a plane is the answer to a
// trained model reading every frame of a real media file, and driving one is
// `lumit-render`'s own job. What the *engine* does with a run is asserted in
// Rust, in `crates/lumit-bridge/src/api/tests.rs`. What is asserted here is
// what this side does: draw a button per Action row, carry a press to the
// engine, have words for every reason it can refuse with, say which of the two
// answers a card is about, and ask for the reading once when the card appears
// rather than once per rebuild.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/plane_display_frb.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/planes.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  /// A comp with one footage layer carrying an enabled `name`, selected.
  ({LumitState state, LumitUiState uiState, LayerReference layer}) withLayer(
    String name,
  ) {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Scene');
    final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
    comp.addFootageLayer(footage: footage, asSequence: false);
    final layer = comp.getLayers().single;
    layer.addEffect(name: name);
    p.uiState
      ..setSelectedComp(comp)
      ..selectedLayer.value = layer;
    p.uiState.model.refresh();
    return (state: p.state, uiState: p.uiState, layer: layer);
  }

  /// A reading of the status, written down: the engine cannot be made to
  /// produce one from Dart, and what this side does with one is the claim.
  BridgePlaneStatus read({
    required BridgePlaneStage stage,
    int done = 0,
    int total = 0,
    BridgePlaneFailure? failure,
    String provider = '',
    int? firstFrame,
    int? lastFrame,
    int clipFrames = 0,
  }) =>
      BridgePlaneStatus(
        stage: stage,
        done: done,
        total: total,
        failure: failure,
        provider: provider,
        firstFrame: firstFrame,
        lastFrame: lastFrame,
        clipFrames: clipFrames,
      );

  group('Depth (frb)', () {
    testWidgets('an Action row is a button, and pressing Cancel is an event',
        (tester) async {
      final p = withLayer('depth');
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      final effect = p.layer.getEffects().single.id();
      final analyse = find.byKey(ValueKey<String>('fx-action-$effect-analyse'));
      final cancel = find.byKey(ValueKey<String>('fx-action-$effect-cancel'));
      expect(analyse, findsOneWidget,
          reason: 'an Action parameter draws a button, not a value field');
      expect(cancel, findsOneWidget);
      // The button says its own name; the row's name column is left empty.
      expect(find.text('Analyse'), findsOneWidget);

      // The status line is there from the start, saying the truth.
      expect(find.byKey(const ValueKey('fx-depth-status')), findsOneWidget);
      expect(find.text('Not analysed yet'), findsOneWidget);

      // Pressing is an **event**: no undo entry, nothing written. What Analyse
      // answers here depends on the machine. No model installed is a refusal,
      // an installed one still has no media at C:/clips to open, and neither
      // may throw out of the button.
      final before = p.state.project!.isDirty();
      await tester.tap(analyse);
      await tester.pump();
      expect(p.state.project!.isDirty(), before,
          reason: 'a press is an event, not an edit');

      // Cancel *is* accepted with nothing running, and the engine records it,
      // which is how this proves the press reached the engine at all and that
      // the status row re-reads on a press. Without the wiring the line would
      // still say "Not analysed yet". Nothing was read, so the honest reading
      // of a stopped run is that there is no depth.
      await tester.tap(cancel);
      await tester.pump();
      expect(find.text('No depth yet'), findsOneWidget);
      expect(find.text('Not analysed yet'), findsNothing);
      expect(p.state.project!.isDirty(), before,
          reason: 'and neither press is an edit');
    });

    testWidgets('the reading is asked for once per press, not once per rebuild',
        (tester) async {
      final p = withLayer('depth');
      final effect = p.layer.getEffects().single.id();
      // A one-slot counter rather than a local, so the test can watch it keep
      // not moving.
      final asked = <int>[0];
      final pressed = ValueNotifier<int>(0);
      addTearDown(pressed.dispose);

      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        child: ValueListenableBuilder<int>(
          valueListenable: pressed,
          builder: (context, n, _) => PlaneDisplayFrb(
            card: PlaneCard.depth,
            layer: p.layer,
            effectId: effect,
            onChanged: () {},
            pressed: n,
            fetch: () {
              asked[0] += 1;
              return read(stage: BridgePlaneStage.idle);
            },
          ),
        ),
      ));
      await tester.pump();
      await tester.pump();
      expect(asked[0], 1, reason: 'one read after the card is up');

      // Rebuild the card without pressing anything: the number must not move,
      // which is what the bridge-call budget exists to protect. The reading is
      // idle, so there is no clock running either.
      for (var i = 0; i < 6; i++) {
        pressed.notifyListeners();
        await tester.pump();
      }
      expect(asked[0], 1, reason: 'a rebuild is not a press');

      // A press is: one bump, one read.
      pressed.value = 1;
      await tester.pump();
      expect(asked[0], 2);
      pressed.value = 2;
      await tester.pump();
      expect(asked[0], 3);
    });

  }, skip: !engineAvailable);
}
