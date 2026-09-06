// The Roto brush's seam: strokes down, the status up, and the words a refusal
// is shown in (docs/08 §3.96).
//
// Every document operation here is genuine; see frb_test_support.dart. What is
// *not* genuine is a propagated matte, and it cannot be: a matte is the answer
// to a minute of decoding and solving a real media file, and driving one is
// `lumit-render`'s own job (docs/impl/roto.md §5). What the *engine* does with a
// run is asserted in Rust. What is asserted here is what this side does: carry a
// scribble into the document as one undoable edit, read it back, and have words
// for every reason the engine can refuse with.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/roto_display_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/roto.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Roto brush (frb)', () {
    /// A comp with one footage layer carrying an enabled Roto brush.
    ({LayerReference layer, BridgeEffectInstance brush}) withBrush() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'roto_brush');
      return (layer: layer, brush: layer.getEffects().single);
    }

    testWidgets('a stroke reaches the document and sets the base frame', (tester) async {
      final w = withBrush();
      final brush = w.brush;
      // Nothing drawn yet: no base frame, so Propagate has nothing to answer
      // with, and the status says so rather than offering a dead button.
      expect(brush.rotoStrokes(), isEmpty);
      expect(rotoStatus(layer: w.layer, effect: brush.id()).baseFrame, isNull);


      final id = brush.id();
      brush.rotoAddStroke(
        points: const [10, 20, 40, 20],
        radius: 6,
        kind: BridgeRotoStrokeKind.foreground,
        frame: 7,
      );
      // Staged, not committed: the commit is the ordinary whole-stack one, so a
      // scribble is one undo step like any other effect edit. The handle is
      // spent by the commit, so the read below takes a fresh one off the layer.
      w.layer.setEffects(effects: [brush]);

      final back = w.layer.getEffects().single.rotoStrokes();
      expect(back, hasLength(1));
      expect(back.single.points, [10, 20, 40, 20]);
      expect(back.single.radius, 6);
      expect(back.single.kind, BridgeRotoStrokeKind.foreground);
      expect(back.single.frame, 7);

      final status = rotoStatus(layer: w.layer, effect: id);
      expect(status.baseFrame, 7, reason: 'the first stroke sets the base frame');
      expect(status.strokes, 1);
      expect(status.stage, BridgeRotoStage.idle,
          reason: 'nothing has been propagated, and idle is the honest reading');
      expect(status.firstFrame, isNull, reason: 'no run means no span to claim');
    });

    testWidgets('a second stroke elsewhere is a correction, not a new base', (tester) async {
      final w = withBrush();
      final brush = w.brush;
      final id = brush.id();
      brush.rotoAddStroke(
        points: const [10, 20, 40, 20],
        radius: 6,
        kind: BridgeRotoStrokeKind.foreground,
        frame: 3,
      );
      brush.rotoAddStroke(
        points: const [50, 60, 70, 60],
        radius: 4,
        kind: BridgeRotoStrokeKind.background,
        frame: 40,
      );
      w.layer.setEffects(effects: [brush]);
      final status = rotoStatus(layer: w.layer, effect: id);
      expect(status.baseFrame, 3,
          reason: 'the base stays where the first stroke put it');
      expect(status.strokes, 2);
    });

    testWidgets('a stroke with no points, or an odd list, is refused', (tester) async {
      final w = withBrush();
      expect(
        () => w.brush.rotoAddStroke(
          points: const [],
          radius: 4,
          kind: BridgeRotoStrokeKind.foreground,
          frame: 0,
        ),
        throwsA(anything),
      );
      expect(
        () => w.brush.rotoAddStroke(
          points: const [1, 2, 3],
          radius: 4,
          kind: BridgeRotoStrokeKind.foreground,
          frame: 0,
        ),
        throwsA(anything),
      );
    });

    testWidgets('clearing takes the strokes and the base with it', (tester) async {
      final w = withBrush();
      final brush = w.brush;
      final id = brush.id();
      brush.rotoAddStroke(
        points: const [1, 1, 5, 5],
        radius: 3,
        kind: BridgeRotoStrokeKind.foreground,
        frame: 2,
      );
      brush.rotoClear();
      w.layer.setEffects(effects: [brush]);
      final status = rotoStatus(layer: w.layer, effect: id);
      expect(status.strokes, 0);
      expect(status.baseFrame, isNull);
    });

    testWidgets('the failure sentence is chosen here, not sent by the engine', (tester) async {
      // Every reason has words. The switch is exhaustive over the generated
      // enum, so this is the check that none of them was left as a blank.
      for (final failure in BridgeRotoFailure.values) {
        expect(rotoFailureSentence(failure).trim(), isNotEmpty);
      }
    });
  });
}
