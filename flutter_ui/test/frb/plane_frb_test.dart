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
import 'package:lumit_flutter/l10n/engine_labels.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/plane_display_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
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

    testWidgets('a partial reading says how far it got, and draws the span',
        (tester) async {
      // A whole shot reports its length; a part of one reports its reach, which
      // is the fact that decides what the user does next.
      expect(
        planeStatusSentence(
            PlaneCard.depth,
            read(
              stage: BridgePlaneStage.done,
              firstFrame: 0,
              lastFrame: 49,
              clipFrames: 50,
            )),
        contains('50 frames'),
      );
      final partial = planeStatusSentence(
          PlaneCard.depth,
          read(
            stage: BridgePlaneStage.done,
            firstFrame: 0,
            lastFrame: 19,
            clipFrames: 50,
          ));
      expect(partial, contains('0'));
      expect(partial, contains('19'));

      // A cancelled run reads exactly like a finished one: the frames it got to
      // are correct and are kept.
      expect(
        planeStatusSentence(
            PlaneCard.depth,
            read(
              stage: BridgePlaneStage.cancelled,
              firstFrame: 0,
              lastFrame: 19,
              clipFrames: 50,
            )),
        partial,
      );
      expect(
        planeCoveredFrames(read(
          stage: BridgePlaneStage.done,
          firstFrame: 0,
          lastFrame: 19,
          clipFrames: 50,
        )),
        20,
      );

      // And the bar under the line is those two counts, in two weights.
      final p = withLayer('depth');
      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        child: PlaneDisplayFrb(
          card: PlaneCard.depth,
          layer: p.layer,
          effectId: p.layer.getEffects().single.id(),
          onChanged: () {},
          pressed: 0,
          fetch: () => read(
            stage: BridgePlaneStage.done,
            provider: 'DirectML',
            firstFrame: 0,
            lastFrame: 19,
            clipFrames: 50,
          ),
        ),
      ));
      await tester.pump();
      await tester.pump();

      expect(
        tester
            .widgetList<Expanded>(find.byType(Expanded))
            .map((e) => e.flex)
            .toList(),
        [20, 30],
        reason: 'the read span and the remainder, in clip frames',
      );
      // A run that fell back to the processor is a run the person waiting
      // should be told about, so the provider is never left unsaid.
      expect(find.byKey(const ValueKey('fx-depth-provider')), findsOneWidget);
      expect(find.text('Read by DirectML'), findsOneWidget);
    });

    testWidgets('the failure sentence is chosen here, not sent by the engine',
        (tester) async {
      // Every reason has words. The switch is exhaustive over the generated
      // enum, so this is the check that none of them was left as a blank.
      for (final failure in BridgePlaneFailure.values) {
        expect(planeFailureSentence(failure).trim(), isNotEmpty);
      }
      // And the two the note keeps apart stay apart: they send the user to two
      // different buttons on the Addons page.
      expect(
        planeFailureSentence(BridgePlaneFailure.runtimeMissing),
        isNot(planeFailureSentence(BridgePlaneFailure.packMissing)),
      );
      // A refusal reaches the line, and the reason is the engine's.
      expect(
        planeStatusSentence(
            PlaneCard.depth,
            read(
              stage: BridgePlaneStage.failed,
              failure: BridgePlaneFailure.packMissing,
            )),
        planeFailureSentence(BridgePlaneFailure.packMissing),
      );
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

    testWidgets('an effect with no model wears the calm badge', (tester) async {
      // The words exist, which is the half of this that is true on every
      // machine. `engine_labels_test` holds the engine's key list against the
      // table; this holds the table against the row that draws it.
      expect(effectBadge('addon_missing'), isNotNull);
      expect(hasEffectBadge('addon_missing'), isTrue);

      final p = withLayer('depth');
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      await tester.pump();

      // The other half is the machine's. Whether the pack is installed is a
      // folder under the user's local data that the engine owns and a widget
      // test cannot move, so both endings are written down: with no model the
      // badge is drawn and names the pack in its detail slot, and with one the
      // effect wears nothing at all.
      final id = p.layer.getEffects().single.id();
      final info = p.layer.getEffects().single.getInfo();
      final badge = find.byKey(ValueKey<String>('fx-badge-$id'));
      if (info.badgeReason == 'addon_missing') {
        expect(badge, findsOneWidget);
        expect(find.text(effectBadge('addon_missing')!), findsOneWidget);
        expect(info.badgeDetail, isNotNull,
            reason: 'the detail names the addon to install');
      } else {
        expect(info.badgeReason, isNull,
            reason: 'a model this machine has is not a reason to wear a badge');
        expect(badge, findsNothing);
      }
    });
  }, skip: !engineAvailable);

  group('Remove background (frb)', () {
    testWidgets('the same two buttons, and a stopped run has no matte',
        (tester) async {
      final p = withLayer('remove_background');
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();

      final effect = p.layer.getEffects().single.id();
      final analyse = find.byKey(ValueKey<String>('fx-action-$effect-analyse'));
      final cancel = find.byKey(ValueKey<String>('fx-action-$effect-cancel'));
      expect(analyse, findsOneWidget);
      expect(cancel, findsOneWidget);

      // The card is registered for this effect too, and it is the matte one:
      // a panel arm left unwritten draws no card at all, and one pointed at
      // the wrong half of the tier says "depth" about a matte.
      expect(find.byKey(const ValueKey('fx-matte-status')), findsOneWidget);
      expect(find.byKey(const ValueKey('fx-depth-status')), findsNothing);
      expect(find.text('Not analysed yet'), findsOneWidget);

      // Cancel is accepted with nothing running and the engine records it,
      // which is how this proves the press reached the engine through the same
      // doorway Depth's does. Nothing was read, so the honest reading is that
      // there is no matte.
      final before = p.state.project!.isDirty();
      await tester.tap(cancel);
      await tester.pump();
      expect(find.text('No matte yet'), findsOneWidget);
      expect(find.text('No depth yet'), findsNothing);
      expect(p.state.project!.isDirty(), before,
          reason: 'a press is an event, not an edit');
    });

    testWidgets('one card, and one word between the two of them',
        (tester) async {
      // Every reading either card can hold, side by side. The progress, the
      // span and the refusals are the tier's and are the same sentence for
      // both; the only thing a matte says differently is what a run that kept
      // nothing left behind.
      final readings = [
        read(stage: BridgePlaneStage.idle),
        read(stage: BridgePlaneStage.queued),
        read(stage: BridgePlaneStage.solving, done: 7, total: 90),
        read(
            stage: BridgePlaneStage.done,
            firstFrame: 0,
            lastFrame: 89,
            clipFrames: 90),
        read(
            stage: BridgePlaneStage.cancelled,
            firstFrame: 0,
            lastFrame: 19,
            clipFrames: 90),
        read(
            stage: BridgePlaneStage.failed,
            failure: BridgePlaneFailure.packMissing),
      ];
      for (final status in readings) {
        expect(planeStatusSentence(PlaneCard.matte, status).trim(), isNotEmpty);
        expect(
          planeStatusSentence(PlaneCard.matte, status),
          planeStatusSentence(PlaneCard.depth, status),
        );
      }
      final nothing = read(stage: BridgePlaneStage.cancelled, clipFrames: 90);
      expect(
        planeStatusSentence(PlaneCard.matte, nothing),
        isNot(planeStatusSentence(PlaneCard.depth, nothing)),
        reason: 'a run that read nothing has no matte, not no depth',
      );
      expect(planeStatusSentence(PlaneCard.matte, nothing).trim(), isNotEmpty);
    });

    testWidgets('Detail is the one model\'s own control', (tester) async {
      // BiRefNet reads a square of its own whatever the frame is, so Detail
      // means nothing to it. The rule is the engine's; this is the check that
      // the panel reads it off this effect's schema rather than leaving a row
      // that does nothing.
      expect(
        disabledParams('remove_background', const {
          'model': BridgeEffectValue.choice(0),
        }),
        isNot(contains('detail')),
        reason: 'Robust Video Matting is the model Detail belongs to',
      );
      expect(
        disabledParams('remove_background', const {
          'model': BridgeEffectValue.choice(1),
        }),
        contains('detail'),
      );

      // And the row is drawn, greyed or not, so there is something to grey.
      final p = withLayer('remove_background');
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      final effect = p.layer.getEffects().single.id();
      expect(find.byKey(ValueKey<String>('fx-row-$effect-detail')),
          findsOneWidget);
      expect(find.text('Detail'), findsOneWidget);
    });
  }, skip: !engineAvailable);
}
