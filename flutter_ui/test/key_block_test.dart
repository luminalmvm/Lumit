// The block tools' arithmetic: what a selection measures, where a
// stretch handle puts each key, and what Reverse and Stagger do to a time.
//
// Pure, so none of it needs a widget tree or the engine — the same bargain
// `easing_curve_test.dart` and `graph_maths_test.dart` strike. The gestures
// that stand on this are tested through the panel in
// `test/frb/timeline_panel_frb_test.dart`; what is claimed here is the
// arithmetic they all share.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/key_block.dart';

void main() {
  group('KeyStretch', () {
    /// The claim the whole gesture rests on: the anchored end does not move,
    /// the dragged end lands where it was put, and everything between keeps
    /// its share of the span.
    test('scales every key proportionally between the two ends', () {
      // 0 … 100, dragged from 100 out to 200: everything doubles.
      const s = KeyStretch(keys: {}, anchor: 0, from: 100, to: 200);
      expect(s.frameOf(0, whole: false), 0, reason: 'the anchor stays put');
      expect(s.frameOf(100, whole: false), 200,
          reason: 'the dragged end lands where it was dragged');
      expect(s.frameOf(25, whole: false), 50);
      expect(s.frameOf(50, whole: false), 100,
          reason: 'a key a quarter along is still a quarter along');
    });

    /// Whole-frame snapped exactly as a single key's drag is with the magnet
    /// on: the block is a gesture on keys, not a new kind of thing.
    test('lands on whole frames when the magnet is on', () {
      const s = KeyStretch(keys: {}, anchor: 0, from: 30, to: 41);
      final loose = s.frameOf(10, whole: false);
      expect(loose, closeTo(13.667, 0.001), reason: 'exact without the magnet');
      expect(s.frameOf(10, whole: true), 14, reason: 'rounded with it');
    });

    /// A block whose ends started on one frame has no span to scale, and
    /// dividing by it is how a stretch becomes infinities.
    test('a zero-span block scales by one rather than by infinity', () {
      const s = KeyStretch(keys: {}, anchor: 40, from: 40, to: 90);
      expect(s.scale, 1);
      expect(s.frameOf(40, whole: false), 40);
    });
  });

  group('clampStretch', () {
    /// A handle dragged onto its anchor would ask for a curve with two keys on
    /// one time, which the engine must refuse; one dragged past it would
    /// invert the block, which is Reverse's job.
    test('keeps a minimum span on the side the end started', () {
      expect(clampStretch(anchor: 0, from: 100, to: 50), 50,
          reason: 'well inside the bound, untouched');
      expect(clampStretch(anchor: 0, from: 100, to: 0), minBlockSpan);
      expect(clampStretch(anchor: 0, from: 100, to: -40), minBlockSpan,
          reason: 'never through the anchor');
    });
  });

  group('reversedFrames', () {
    /// Returned in the order it was given, so each new time pairs with the key
    /// it belongs to — which is what makes the value travel with its key.
    test('answers in the order it was asked', () {
      expect(reversedFrames([40, 10, 20]), [10, 40, 30]);
    });
  });

  group('staggeredFrame', () {
    test('pushes each row one step further than the row above', () {
      double at(int rank) => staggeredFrame(100,
          rank: rank, rows: 3, step: 4, order: StaggerOrder.topDown);
      expect(at(0), 100, reason: 'the top row keeps its timing');
      expect(at(1), 104);
      expect(at(2), 108);
    });
  });
}
