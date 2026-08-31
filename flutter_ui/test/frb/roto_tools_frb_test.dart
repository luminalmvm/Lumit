// The Roto tools over the picture, and the Roto brush's status row
// (K-717, docs/impl/roto.md §10 item 9, docs/07 §2.3.7).
//
// Every document operation here is genuine — the strokes really land in a real
// project through the real bridge. What is handed in are the two answers a test
// machine cannot have: which frame of a file is on screen (there is no file),
// and a propagated matte's edge (a matte is a minute of decoding and solving,
// which is `lumit-render`'s own job). Both are the seam `ViewerTrackLayer.fetch`
// already is, and both are asserted *through* rather than *about*.

import 'dart:typed_data';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/camera_track_display_frb.dart'
    show TrackSpanBar;
import 'package:lumit_flutter/panels/roto_display_frb.dart';
import 'package:lumit_flutter/panels/viewer_gizmo.dart';
import 'package:lumit_flutter/panels/viewer_layer_map.dart';
import 'package:lumit_flutter/panels/viewer_roto.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/roto.dart';
import 'package:lumit_flutter/state/tools.dart';
import 'package:uuid/uuid.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  /// A project with one footage layer carrying an enabled Roto brush, selected.
  ({
    LumitState state,
    LumitUiState uiState,
    LayerReference layer,
    UuidValue effect,
  }) withBrush() {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Scene');
    final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
    comp.addFootageLayer(footage: footage, asSequence: false);
    final layer = comp.getLayers().single;
    layer.addEffect(name: 'roto_brush');
    final effect = layer.getEffects().single.id();
    p.uiState
      ..setSelectedComp(comp)
      ..selectedLayer.value = layer;
    return (
      state: p.state,
      uiState: p.uiState,
      layer: layer,
      effect: effect,
    );
  }

  /// The layer as the Viewer maps it, drawn at **half** magnification with its
  /// origin on the panel's origin.
  ///
  /// Half deliberately: an identity map would pass whether the gesture went
  /// through the comp→layer chain or straight through, and going straight
  /// through is exactly the mistake K-248 exists to stop. At this scale a point
  /// on the panel is twice as far into the file.
  LayerBox boxFor(LayerReference layer) => LayerBox(
        layer: layer,
        id: layer.internallayerId,
        map: ViewerLayerMap.of(
          positionX: 0,
          positionY: 0,
          anchorX: 0,
          anchorY: 0,
          scaleXPercent: 100,
          scaleYPercent: 100,
          rotationDegrees: 0,
          origin: Offset.zero,
          viewScale: 0.5,
        ),
        bounds: const Size(1920, 1080),
        draggable: true,
        scalable: true,
        rotationDegrees: 0,
      );

  /// Mount the overlay on its own, with the two engine answers handed in.
  ///
  /// [frame] and [nudge] are separate on purpose: bumping `nudge` rebuilds the
  /// overlay without changing anything it draws from, which is what makes "once
  /// per frame, not once per rebuild" a countable claim.
  Future<void> mountOverlay(
    WidgetTester tester,
    ({
      LumitState state,
      LumitUiState uiState,
      LayerReference layer,
      UuidValue effect,
    }) w, {
    required ValueNotifier<int> frame,
    required ValueNotifier<int> nudge,
    int view = 0,
    ToolMode tool = ToolMode.rotoBrush,
    int Function(LayerReference, int)? sourceFrameOf,
    Float32List Function(UuidValue, int)? boundaryOf,
  }) async {
    w.uiState.tools.select(tool);
    await tester.pumpWidget(hostPanel(
      state: w.state,
      uiState: w.uiState,
      size: const Size(500, 400),
      child: ValueListenableBuilder<int>(
        valueListenable: nudge,
        builder: (context, _, __) => Stack(
          children: [
            ViewerRotoLayer(
              active: true,
              tool: tool,
              state: w.state,
              uiState: w.uiState,
              boxes: [boxFor(w.layer)],
              target: (effect: w.effect, view: view),
              viewScale: 0.5,
              playheadFrame: frame.value,
              revision: w.uiState.model.heldRevision,
              onChanged: w.uiState.model.refresh,
              sourceFrameOf: sourceFrameOf ?? (_, f) => f * 2,
              boundaryOf: boundaryOf,
            ),
          ],
        ),
      ),
    ));
    await tester.pumpAndSettle();
  }

  /// Drag across the picture, which is how a scribble is made.
  Future<void> scribble(WidgetTester tester, Offset from) async {
    final gesture = await tester.startGesture(from);
    await tester.pump();
    for (var i = 0; i < 5; i++) {
      await gesture.moveBy(const Offset(12, 0));
      await tester.pump();
    }
    await gesture.up();
    await tester.pumpAndSettle();
  }

  List<BridgeRotoStroke> strokesOf(LayerReference layer) =>
      layer.getEffects().single.rotoStrokes();

  group('The Roto tools over the picture (frb)', () {
    testWidgets('a scribble lands as a document stroke in source pixels',
        (tester) async {
      final w = withBrush();
      final frame = ValueNotifier(0);
      final nudge = ValueNotifier(0);
      addTearDown(frame.dispose);
      addTearDown(nudge.dispose);
      await mountOverlay(tester, w, frame: frame, nudge: nudge);

      expect(strokesOf(w.layer), isEmpty);
      await scribble(tester, const Offset(100, 80));

      final strokes = strokesOf(w.layer);
      expect(strokes, hasLength(1), reason: 'one drag, one stroke');
      final stroke = strokes.single;
      expect(stroke.kind, BridgeRotoStrokeKind.foreground,
          reason: 'a plain drag claims the subject');
      expect(stroke.points.length, greaterThanOrEqualTo(4),
          reason: 'the path the pointer took, thinned, not one dab');
      expect(stroke.points.length.isEven, isTrue, reason: 'x, y pairs');
      // Half magnification, so the panel's (100, 80) is the file's (200, 160).
      // This is the whole of K-248 in one assertion: the number stored is the
      // file's own pixel, not the panel's and not the composition's.
      expect(stroke.points[0], closeTo(200, 1));
      expect(stroke.points[1], closeTo(160, 1));
      // The last point is where the pointer stopped, five 12-px steps along.
      expect(stroke.points[stroke.points.length - 2], closeTo(320, 2));
      // And the width is in that same ruler.
      expect(stroke.radius, w.uiState.tools.rotoSize / 2);
    });

    testWidgets('Alt claims the background instead', (tester) async {
      final w = withBrush();
      final frame = ValueNotifier(0);
      final nudge = ValueNotifier(0);
      addTearDown(frame.dispose);
      addTearDown(nudge.dispose);
      await mountOverlay(tester, w, frame: frame, nudge: nudge);

      await tester.sendKeyDownEvent(LogicalKeyboardKey.altLeft);
      await scribble(tester, const Offset(60, 60));
      await tester.sendKeyUpEvent(LogicalKeyboardKey.altLeft);

      expect(strokesOf(w.layer).single.kind, BridgeRotoStrokeKind.background);
    });

    testWidgets('the Refine edge tool claims the refine band', (tester) async {
      final w = withBrush();
      final frame = ValueNotifier(0);
      final nudge = ValueNotifier(0);
      addTearDown(frame.dispose);
      addTearDown(nudge.dispose);
      await mountOverlay(tester, w,
          frame: frame, nudge: nudge, tool: ToolMode.refineEdge);

      await scribble(tester, const Offset(60, 60));
      expect(strokesOf(w.layer).single.kind, BridgeRotoStrokeKind.refine);
    });

    /// The base frame is a **source** frame, and the engine is what maps one to
    /// the other — the layer's start offset and its Retime map both live in the
    /// document. The handed-in mapping doubles the composition frame, so a base
    /// that came from the playhead instead would read 9 rather than 18.
    testWidgets('the first scribble sets the base to the source frame',
        (tester) async {
      final w = withBrush();
      final frame = ValueNotifier(9);
      final nudge = ValueNotifier(0);
      addTearDown(frame.dispose);
      addTearDown(nudge.dispose);
      await mountOverlay(tester, w, frame: frame, nudge: nudge);

      expect(rotoStatus(layer: w.layer, effect: w.effect).baseFrame, isNull);
      await scribble(tester, const Offset(70, 70));

      final status = rotoStatus(layer: w.layer, effect: w.effect);
      expect(status.baseFrame, 18, reason: 'the file\'s frame, not the comp\'s');
      expect(strokesOf(w.layer).single.frame, 18);

      // A second scribble on another frame is a correction and leaves the base
      // where the first one put it.
      frame.value = 40;
      nudge.value++;
      await tester.pumpAndSettle();
      await scribble(tester, const Offset(90, 120));
      expect(rotoStatus(layer: w.layer, effect: w.effect).baseFrame, 18);
      expect(strokesOf(w.layer).last.frame, 80);
    });

    /// K-681, and `bridge_call_budget_test` is the gate: the overlay's answers
    /// are held against the frame, the document and a propagation landing, so a
    /// rebuild draws them again and asks nothing.
    testWidgets('the edge is read once per frame and not once per rebuild',
        (tester) async {
      final w = withBrush();
      final frame = ValueNotifier(3);
      final nudge = ValueNotifier(0);
      addTearDown(frame.dispose);
      addTearDown(nudge.dispose);
      var reads = 0;
      await mountOverlay(
        tester,
        w,
        frame: frame,
        nudge: nudge,
        view: rotoViewBoundary,
        boundaryOf: (_, f) {
          reads++;
          return Float32List.fromList([10, 10, 20, 20]);
        },
      );
      expect(reads, 1, reason: 'once, when the overlay went up');

      for (var i = 0; i < 12; i++) {
        nudge.value++;
        await tester.pump();
      }
      expect(reads, 1, reason: 'twelve rebuilds asked the store nothing');

      frame.value = 4;
      nudge.value++;
      await tester.pumpAndSettle();
      expect(reads, 2, reason: 'and a new frame is a new answer');
    });

    /// Every view but Boundary is a picture the *stack* draws, so scanning a
    /// matte for an outline nobody is showing would be work for nothing.
    testWidgets('the edge is not read at all in the Result view',
        (tester) async {
      final w = withBrush();
      final frame = ValueNotifier(0);
      final nudge = ValueNotifier(0);
      addTearDown(frame.dispose);
      addTearDown(nudge.dispose);
      var reads = 0;
      await mountOverlay(tester, w, frame: frame, nudge: nudge, boundaryOf: (
        _,
        __,
      ) {
        reads++;
        return Float32List(0);
      });
      frame.value = 5;
      nudge.value++;
      await tester.pumpAndSettle();
      expect(reads, 0);
    });

    /// The refusals, each with words: no layer, and a layer with no Roto brush.
    testWidgets('a scribble with nothing selected says so and stores nothing',
        (tester) async {
      final w = withBrush();
      final frame = ValueNotifier(0);
      final nudge = ValueNotifier(0);
      addTearDown(frame.dispose);
      addTearDown(nudge.dispose);
      w.uiState.selectedLayer.value = null;
      await mountOverlay(tester, w, frame: frame, nudge: nudge);

      await scribble(tester, const Offset(80, 80));
      expect(strokesOf(w.layer), isEmpty);
      expect(w.state.notice.value, isNotNull);
      expect(w.state.notice.value!.message.trim(), isNotEmpty);
    });
  });

  group('The Roto brush status row (frb)', () {
    BridgeRotoStatus status({
      BridgeRotoStage stage = BridgeRotoStage.done,
      int? first,
      int? last,
      int clipFrames = 0,
      int? base,
      int strokes = 0,
    }) =>
        BridgeRotoStatus(
          stage: stage,
          done: 0,
          total: 0,
          reused: 0,
          firstFrame: first,
          lastFrame: last,
          clipFrames: clipFrames,
          baseFrame: base,
          strokes: strokes,
        );

    Future<void> mountRow(
      WidgetTester tester,
      ({
        LumitState state,
        LumitUiState uiState,
        LayerReference layer,
        UuidValue effect,
      }) w,
      BridgeRotoStatus reading,
    ) async {
      await tester.pumpWidget(hostPanel(
        state: w.state,
        uiState: w.uiState,
        size: const Size(340, 200),
        child: RotoDisplayFrb(
          layer: w.layer,
          effectId: w.effect,
          playheadFrame: 0,
          onChanged: () {},
          pressed: 0,
          fetch: () => reading,
        ),
      ));
      await tester.pumpAndSettle();
    }

    /// The K-540 bar: the span the matte covers against the length of the clip,
    /// which is two frame counts and nothing else.
    testWidgets('the span bar weighs the two frame counts', (tester) async {
      final w = withBrush();
      await mountRow(
        tester,
        w,
        status(first: 10, last: 59, clipFrames: 200, base: 10, strokes: 1),
      );
      final bar = tester.widget<TrackSpanBar>(
          find.byKey(const ValueKey('fx-roto-span')));
      expect(bar.analysed, 50, reason: 'frames 10 to 59 inclusive');
      expect(bar.total, 200);
      expect(find.byKey(const ValueKey('fx-roto-base')), findsOneWidget);
      expect(find.byKey(const ValueKey('fx-roto-assign-base')), findsOneWidget);
    });

    /// Before anything is propagated the bar is entirely the surface tone —
    /// none of this shot is cut yet, said honestly rather than by being absent.
    testWidgets('nothing propagated weighs nothing', (tester) async {
      final w = withBrush();
      await mountRow(
        tester,
        w,
        status(stage: BridgeRotoStage.idle, clipFrames: 120, strokes: 2),
      );
      expect(
        rotoCoveredFrames(status(clipFrames: 120)),
        0,
      );
      expect(
        tester
            .widget<TrackSpanBar>(
                find.byKey(const ValueKey('fx-roto-span')))
            .analysed,
        0,
      );
    });

    /// A brush nobody has scribbled on has no base to move, so the row that
    /// would move it is not offered.
    testWidgets('the base row appears only once there are strokes',
        (tester) async {
      final w = withBrush();
      await mountRow(tester, w, status(stage: BridgeRotoStage.idle));
      expect(find.byKey(const ValueKey('fx-roto-base')), findsNothing);
      expect(find.byKey(const ValueKey('fx-roto-status')), findsOneWidget);
    });
  });
}
