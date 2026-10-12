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
  });
}
