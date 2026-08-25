// The Camera track effect's interface: its buttons, its status line, its point
// cloud, and the badge a solve-linked Camera layer wears (K-417).
//
// Every document operation here is genuine; see frb_test_support.dart. What is
// *not* genuine is the solve behind the point cloud, and it cannot be: a solve
// is the answer to a minutes-long analysis of a real media file, and driving one
// is `lumit-render`'s own job (docs/impl/tracking.md §5b). What the *engine*
// does with a solve — where a point lands, where a Null goes, what the bake
// writes — is asserted in Rust, in `crates/lumit-bridge/src/api/tests.rs`. What
// is asserted here is what this side does: draws a dot per point, picks them,
// and asks the engine once per frame rather than once per rebuild.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/camera_track_display_frb.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/panels/viewer_track.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/track.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Camera track (frb)', () {
    /// A comp with one footage layer carrying an enabled Camera track, selected.
    ({LumitState state, LumitUiState uiState, LayerReference layer})
        withTrackedLayer() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'camera_track');
      p.uiState
        ..setSelectedComp(comp)
        ..selectedLayer.value = layer;
      p.uiState.model.refresh();
      return (state: p.state, uiState: p.uiState, layer: layer);
    }

    testWidgets('an Action row is a button, and pressing it is an event',
        (tester) async {
      final p = withTrackedLayer();
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
      expect(
          find.byKey(const ValueKey('fx-camera-track-status')), findsOneWidget);
      expect(find.text('Not analysed yet'), findsOneWidget);

      // Pressing is an **event**: no undo entry, nothing written. This
      // fixture's media does not exist, so Analyse is refused — and a refusal
      // must not throw out of the button.
      final before = p.state.project!.isDirty();
      await tester.tap(analyse);
      await tester.pump();
      expect(p.state.project!.isDirty(), before,
          reason: 'a press is an event, not an edit');

      // Cancel *is* accepted with nothing running, and the engine records it —
      // which is how this proves the press reached the engine at all and that
      // the status row re-reads on a press. Without the wiring the line would
      // still say "Not analysed yet".
      await tester.tap(cancel);
      await tester.pump();
      expect(find.text('Analysis stopped'), findsOneWidget);
      expect(find.text('Not analysed yet'), findsNothing);
      expect(p.state.project!.isDirty(), before,
          reason: 'and neither press is an edit');
    });

    testWidgets('the failure sentence is chosen here, not sent by the engine',
        (tester) async {
      // Every reason has words. The switch is exhaustive over the generated
      // enum, so this is the check that none of them was left as a blank.
      for (final failure in BridgeTrackFailure.values) {
        expect(trackFailureSentence(failure).trim(), isNotEmpty);
      }
    });

    /// A cloud that does not depend on a solve existing — see the file header.
    List<BridgeTrackPoint> cloud() => const [
          BridgeTrackPoint(track: 1, x: 100, y: 100, depth: 1.0),
          BridgeTrackPoint(track: 2, x: 300, y: 100, depth: 0.5),
          BridgeTrackPoint(track: 3, x: 500, y: 400, depth: 0.0),
        ];

    /// The overlay alone, over a picture that fills the panel one comp pixel to
    /// one panel pixel, so a point's composition coordinates are also where to
    /// tap.
    Future<List<int>> mountCloud(
      WidgetTester tester,
      ({LumitState state, LumitUiState uiState, LayerReference layer}) p, {
      ValueNotifier<int>? frame,
    }) async {
      // A one-slot counter rather than a local, so the caller can watch it
      // keep not moving.
      final asked = <int>[0];
      final at = frame ?? ValueNotifier<int>(0);
      await tester.pumpWidget(hostPanel(
        size: const Size(640, 480),
        state: p.state,
        uiState: p.uiState,
        child: ValueListenableBuilder<int>(
          valueListenable: at,
          builder: (context, n, _) => Stack(
            children: [
              // The panel places it; here the whole panel is the picture.
              Positioned.fill(
                  child: ViewerTrackLayer(
                tracked: p.layer,
                selecting: true,
                fitted: const Rect.fromLTWH(0, 0, 640, 480),
                compSize: const Size(640, 480),
                playheadFrame: n,
                revision: null,
                accent: const Color(0xFF00FF00),
                mark: const Color(0xFFFFFFFF),
                onChanged: () {},
                fetch: (_, __) {
                  asked[0] += 1;
                  return cloud();
                },
              )),
            ],
          ),
        ),
      ));
      await tester.pump();
      return asked;
    }

    testWidgets('a dot is drawn per solved point, and a click picks one',
        (tester) async {
      final p = withTrackedLayer();
      await mountCloud(tester, p);

      final painted = tester.widget<CustomPaint>(find.byKey(
        const ValueKey('viewer-track-points'),
      ));
      final painter = painted.painter! as TrackPointPainter;
      expect(painter.points.length, 3);
      expect(painter.points.where((p) => p.picked), isEmpty);
      // The depth cue arrives from the engine and is drawn, not recomputed.
      expect(painter.points.first.depth, 1.0);

      // Nothing is offered until something is picked.
      expect(
          find.byKey(const ValueKey('viewer-track-create-null')), findsNothing);

      await tester.tapAt(const Offset(100, 100));
      await tester.pump();
      final one = (tester
          .widget<CustomPaint>(find.byKey(
            const ValueKey('viewer-track-points'),
          ))
          .painter! as TrackPointPainter);
      expect(one.points.where((p) => p.picked).length, 1);
      expect(find.byKey(const ValueKey('viewer-track-create-null')),
          findsOneWidget);
      expect(find.byKey(const ValueKey('viewer-track-create-solid')),
          findsOneWidget);

      // A click on empty picture clears, which is what a click on nothing
      // means everywhere else.
      await tester.tapAt(const Offset(600, 40));
      await tester.pump();
      expect(
        (tester
                .widget<CustomPaint>(find.byKey(
                  const ValueKey('viewer-track-points'),
                ))
                .painter! as TrackPointPainter)
            .points
            .where((p) => p.picked),
        isEmpty,
      );
    });

    testWidgets('shift adds, a box takes several, and Escape clears',
        (tester) async {
      final p = withTrackedLayer();
      await mountCloud(tester, p);

      List<({Offset at, double depth, bool picked})> drawn() => (tester
              .widget<CustomPaint>(find.byKey(
                const ValueKey('viewer-track-points'),
              ))
              .painter! as TrackPointPainter)
          .points;

      await tester.tapAt(const Offset(100, 100));
      await tester.pump();
      await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.tapAt(const Offset(300, 100));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.pump();
      expect(drawn().where((p) => p.picked).length, 2,
          reason: 'shift-click adds rather than replacing');

      // A box round the two nearer points, dragged on empty picture.
      await tester.dragFrom(
        const Offset(600, 40),
        const Offset(-580, 300),
      );
      await tester.pump();
      expect(drawn().where((p) => p.picked).length, 2);

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pump();
      expect(drawn().where((p) => p.picked), isEmpty);
      expect(
          find.byKey(const ValueKey('viewer-track-create-null')), findsNothing);
    });

    testWidgets('the cloud is asked for once per frame, not once per rebuild',
        (tester) async {
      final p = withTrackedLayer();
      final frame = ValueNotifier<int>(0);
      addTearDown(frame.dispose);
      final asked = await mountCloud(tester, p, frame: frame);
      expect(asked[0], 1, reason: 'one read when the overlay appears');

      // Rebuild the overlay, and select in it, without moving the playhead:
      // the number must not move (K-413's rule, and what the bridge-call
      // budget exists to protect).
      for (var i = 0; i < 6; i++) {
        frame.notifyListeners();
        await tester.pump();
      }
      await tester.tapAt(const Offset(100, 100));
      await tester.pump();
      await tester.tapAt(const Offset(300, 100));
      await tester.pump();
      expect(asked[0], 1, reason: 'a rebuild is not a new frame');

      // A frame is: one move, one read.
      frame.value = 1;
      await tester.pump();
      expect(asked[0], 2);
      frame.value = 2;
      await tester.pump();
      expect(asked[0], 3);
    });

    /// **Switching the effect off takes the cloud with it** (K-430).
    ///
    /// The cloud is found in the read model, and the model changing is a thing
    /// to listen to. It was read outside any listener, so the dots stayed on
    /// the picture after the effect was disabled until the frame next changed —
    /// which, paused, could be never.
    testWidgets('disabling the effect removes the cloud, with no frame change',
        (tester) async {
      final p = withTrackedLayer();
      // Show points is off by default; the cloud is only ever drawn with it on.
      final effects = p.layer.getEffects();
      effects.single.setValue(
          id: 'show_points', value: const BridgeEffectValue.bool(true));
      p.layer.setEffects(effects: effects);
      p.uiState.model.refresh();

      await tester.pumpWidget(hostPanel(
        state: p.state,
        uiState: p.uiState,
        size: const Size(900, 600),
        child: const ViewerPanelFrb(),
      ));
      await settleFrb(tester, minRounds: 8);

      final cloudKey =
          ValueKey<String>('viewer-track-${p.layer.internallayerId}');
      expect(find.byKey(cloudKey), findsOneWidget,
          reason: 'Show points is on, so the cloud is drawn');

      final frame = p.uiState.playheadFrame.value;
      p.layer.setEffectEnabled(
          effect: p.layer.getEffects().single, enabled: false);
      // What the Effect Controls panel does after a switch: re-read. Nothing
      // else moves, and in particular the playhead does not.
      p.uiState.model.refresh();
      await tester.pump();

      expect(find.byKey(cloudKey), findsNothing,
          reason: 'a disabled effect draws nothing');
      expect(p.uiState.playheadFrame.value, frame,
          reason: 'and it took no frame change to notice');
    });

    /// **A solve landing makes the cloud appear** (K-430).
    ///
    /// The read is keyed by the frame and the document's revision, and a solve
    /// moves neither: it is the answer to an analysis, not an edit. Without a
    /// third key the dots did not arrive until something else did.
    testWidgets('a landed solve is read without the playhead moving',
        (tester) async {
      final p = withTrackedLayer();
      var solved = false;
      await tester.pumpWidget(hostPanel(
        size: const Size(640, 480),
        state: p.state,
        uiState: p.uiState,
        child: ValueListenableBuilder<int>(
          valueListenable: p.uiState.solveLanded,
          builder: (context, generation, _) => Stack(children: [
            Positioned.fill(
              child: ViewerTrackLayer(
                tracked: p.layer,
                selecting: true,
                fitted: const Rect.fromLTWH(0, 0, 640, 480),
                compSize: const Size(640, 480),
                playheadFrame: 0,
                revision: null,
                generation: generation,
                accent: const Color(0xFF00FF00),
                mark: const Color(0xFFFFFFFF),
                onChanged: () {},
                // Nothing until the analysis lands, then the cloud — which is
                // what the engine does, one solve later.
                fetch: (_, __) => solved ? cloud() : const <BridgeTrackPoint>[],
              ),
            ),
          ]),
        ),
      ));
      await tester.pump();
      expect(find.byKey(const ValueKey('viewer-track-points')), findsNothing,
          reason: 'nothing is solved yet');

      solved = true;
      // The one thing the Camera track's card does when a solve lands.
      p.uiState.solveLanded.value++;
      await tester.pump();
      await tester.pump();

      final painter = tester
          .widget<CustomPaint>(
            find.byKey(const ValueKey('viewer-track-points')),
          )
          .painter! as TrackPointPainter;
      expect(painter.points.length, 3,
          reason: 'the solve was read without the playhead moving');
      expect(p.uiState.playheadFrame.value, 0);
    });

    testWidgets('a linked camera wears the badge and converts to keyframes',
        (tester) async {
      final p = withTrackedLayer();
      final camera = p.uiState.selectedComp!.addCameraLayer();
      // No solve exists in this process, so the link resolves nowhere — which
      // is a state the badge has to be able to say out loud rather than a
      // reason to draw nothing.
      setCameraSolveLink(
        camera: camera,
        tracked: p.layer.internallayerId,
      );
      p.uiState
        ..selectedLayer.value = camera
        ..setSelection([camera]);
      p.uiState.model.refresh();

      p.uiState.workspace.interface.transformInEffectControls = true;
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state,
        uiState: p.uiState,
      ));
      await tester.pump();
      await tester.pump();

      expect(
          find.byKey(const ValueKey('tf-camera-link-badge')), findsOneWidget);
      expect(cameraLink(camera: camera, frame: 0).state,
          BridgeLinkState.unresolved);

      await tester.tap(find.byKey(const ValueKey('tf-camera-link-convert')));
      await tester.pump();

      // Read back through the model, not through the widget: the claim is that
      // the document changed.
      final after = cameraLink(camera: camera, frame: 0);
      expect(after.state, BridgeLinkState.unlinked);
      expect(after.tracked, isNull);
      expect(camera.getTransform().positionX is BridgeScalar_Keyframed, isTrue,
          reason: 'the bake writes one key per frame');
    });
  }, skip: !engineAvailable);
}
