// The Planar track effect's interface: its three buttons, its status line, and
// the span bar a partial track draws (K-579).
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
import 'package:lumit_flutter/panels/camera_track_display_frb.dart';
import 'package:lumit_flutter/panels/planar_track_display_frb.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/track.dart';

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

    testWidgets('three buttons, and a press reaches the engine',
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
      final cancel = find.byKey(ValueKey<String>('fx-action-$effect-cancel'));
      expect(cancel, findsOneWidget);

      // The quad's four corners are the effect's own rows — each an x/y pair,
      // folded into one point row the way every other pair is (K-443) — and the
      // layer the pin lands on is a row beside them.
      expect(find.text('Upper left'), findsOneWidget);
      expect(find.text('Lower right'), findsOneWidget);
      expect(find.text('Pin layer'), findsOneWidget);

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

    /// A reading, written down — the engine cannot be made to produce one from
    /// Dart, and what this side does with one is the claim.
    BridgePlanarStatus tracked({
      required int frames,
      required int clipFrames,
      int reanchors = 0,
    }) =>
        BridgePlanarStatus(
          stage: BridgeTrackStage.done,
          done: 0,
          total: 0,
          frames: frames,
          clipFrames: clipFrames,
          reanchors: reanchors,
        );

    test('a partial track leads with its span, a whole one with its length',
        () {
      final whole = planarStatusSentence(tracked(frames: 50, clipFrames: 50));
      expect(whole, contains('50'));
      final partial = planarStatusSentence(tracked(frames: 18, clipFrames: 50));
      expect(partial, contains('18'));
      expect(partial, contains('50'));
      expect(partial, isNot(equals(whole)),
          reason: 'a partial track has to say something different');
    });

    testWidgets('the span bar and the re-anchor line appear only when earned',
        (tester) async {
      final p = withPlanarLayer();
      final effect = p.layer.getEffects().single.id();

      /// Mount the display over one written-down reading, in a fresh tree each
      /// time — `hostPanel` puts its child in an `Overlay`, whose entries are
      /// taken once, so re-pumping the same host would keep showing the first
      /// child. A keyed subtree makes each reading its own mount, which is also
      /// the path a real card takes when it is first opened.
      Future<void> show(BridgePlanarStatus? feed, String tag) async {
        await tester.pumpWidget(KeyedSubtree(
          key: ValueKey<String>(tag),
          child: hostPanel(
            child: PlanarTrackDisplayFrb(
              layer: p.layer,
              effectId: effect,
              onChanged: () {},
              pressed: 0,
              fetch: feed == null ? null : () => feed,
            ),
            state: p.state,
            uiState: p.uiState,
          ),
        ));
        // Once for the frame, once for the post-frame reading.
        await tester.pump();
        await tester.pump();
      }

      // Nothing tracked: no bar, and no re-anchor line to explain.
      await show(null, 'idle');
      expect(find.byKey(const ValueKey('fx-planar-track-span')), findsNothing);
      expect(find.byKey(const ValueKey('fx-planar-track-reanchors')),
          findsNothing);

      // A whole track, measured entirely against its reference frame: the bar
      // is drawn and there is still nothing to warn about.
      await show(tracked(frames: 50, clipFrames: 50), 'whole');
      final bar = tester.widget<TrackSpanBar>(
          find.byKey(const ValueKey('fx-planar-track-span')));
      expect(bar.analysed, 50, reason: 'the bar is the two frame counts');
      expect(bar.total, 50);
      expect(find.byKey(const ValueKey('fx-planar-track-reanchors')),
          findsNothing);

      // A partial, re-anchored track says both things.
      await show(tracked(frames: 18, clipFrames: 50, reanchors: 3), 'partial');
      final partial = tester.widget<TrackSpanBar>(
          find.byKey(const ValueKey('fx-planar-track-span')));
      expect(partial.analysed, 18);
      expect(partial.total, 50);
      expect(find.byKey(const ValueKey('fx-planar-track-reanchors')),
          findsOneWidget,
          reason: 'a re-anchored track carries drift, and nothing else says so');
    });
  });
}
