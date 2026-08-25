// The block tools' arithmetic (K-458): what a selection measures, where a
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
  group('KeyBlock', () {
    test('spans from the earliest key to the latest, counted whole', () {
      const block = KeyBlock(first: 12, last: 36, count: 2);
      expect(block.spanFrames, 24, reason: "the badge's second number");
      expect(block.count, 2);
    });

    /// A time that has been through a rational conversion comes back a
    /// thousandth of a frame short; the badge counts frames, not thousandths.
    test('a span three thousandths short still reads whole', () {
      const block = KeyBlock(first: 0, last: 23.997, count: 2);
      expect(block.spanFrames, 24);
    });

    test('one key is not a block', () {
      expect(KeyBlock.isBlock(0), isFalse);
      expect(KeyBlock.isBlock(1), isFalse,
          reason: 'a lone key has its own drag, and would span 0 f');
      expect(KeyBlock.isBlock(2), isTrue);
    });
  });

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

    test('squeezes as readily as it spreads', () {
      const s = KeyStretch(keys: {}, anchor: 0, from: 100, to: 25);
      expect(s.frameOf(100, whole: false), 25);
      expect(s.frameOf(40, whole: false), 10);
    });

    /// Dragging the *earlier* end anchors the later one, which is the other
    /// half of the gesture and the half that is easy to get backwards.
    test('anchors the far end when the near one is dragged', () {
      const s = KeyStretch(keys: {}, anchor: 100, from: 0, to: 50);
      expect(s.frameOf(100, whole: false), 100, reason: 'the far end holds');
      expect(s.frameOf(0, whole: false), 50);
      expect(s.frameOf(50, whole: false), 75,
          reason: 'the middle key follows the half-length span');
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

    test('movedTo keeps the anchor and the origin', () {
      const s = KeyStretch(keys: {'a#0'}, anchor: 0, from: 10, to: 10);
      final moved = s.movedTo(20);
      expect(moved.anchor, 0);
      expect(moved.from, 10);
      expect(moved.to, 20);
      expect(moved.keys, {'a#0'});
    });
  });

  /// The primitive under every scaling gesture on a block: the lane stretch
  /// works in frames, the graph's transform box in frames *and* pixels
  /// (docs/impl/timeline-interaction.md §6.2).
  group('scaledAbout', () {
    test('holds the anchor and lands the dragged end where it was put', () {
      expect(scaledAbout(anchor: 0, from: 100, to: 50, at: 0), 0,
          reason: 'the anchor never moves');
      expect(scaledAbout(anchor: 0, from: 100, to: 50, at: 100), 50,
          reason: 'the end in hand lands exactly there');
      expect(scaledAbout(anchor: 0, from: 100, to: 50, at: 40), 20,
          reason: 'and everything between keeps its share');
    });

    /// Pixels count down the screen where values count up, so a value scale
    /// hands this an anchor *below* its origin — the arithmetic does not care
    /// which way the axis runs.
    test('works with the axis the other way up', () {
      expect(scaledAbout(anchor: 300, from: 100, to: 200, at: 200),
          closeTo(250, 1e-9));
    });

    test('a zero reach scales by one rather than by infinity', () {
      expect(scaledAbout(anchor: 40, from: 40, to: 90, at: 12), 12);
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

    test('and the same the other way round', () {
      expect(clampStretch(anchor: 100, from: 0, to: 120), 100 - minBlockSpan);
      expect(clampStretch(anchor: 100, from: 0, to: 40), 40);
    });
  });

  group('reversedFrames', () {
    /// The block plays backwards **where it stands**: the earliest time goes to
    /// the latest and back, and the run does not move along the Timeline.
    test('mirrors within the selection\'s own span', () {
      expect(reversedFrames([10, 20, 40]), [40, 30, 10]);
    });

    test('leaves the ends where the ends were', () {
      final out = reversedFrames([0, 3, 7, 24]);
      expect(out.reduce((a, b) => a < b ? a : b), 0);
      expect(out.reduce((a, b) => a > b ? a : b), 24);
    });

    /// Returned in the order it was given, so each new time pairs with the key
    /// it belongs to — which is what makes the value travel with its key.
    test('answers in the order it was asked', () {
      expect(reversedFrames([40, 10, 20]), [10, 40, 30]);
    });

    test('reversing twice is where you started', () {
      const frames = [2.0, 5.0, 11.0, 30.0];
      expect(reversedFrames(reversedFrames(frames)), frames);
    });

    test('an empty selection reverses to nothing', () {
      expect(reversedFrames(const []), isEmpty);
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

    test('bottom up counts the same ranks from the other end', () {
      double at(int rank) => staggeredFrame(100,
          rank: rank, rows: 3, step: 4, order: StaggerOrder.bottomUp);
      expect(at(2), 100, reason: 'now the bottom row keeps its timing');
      expect(at(1), 104);
      expect(at(0), 108);
    });

    /// Zero is the popover's resting value, so it has to be the identity
    /// rather than a case anyone has to switch off.
    test('a step of zero moves nothing', () {
      expect(
          staggeredFrame(17,
              rank: 5, rows: 9, step: 0, order: StaggerOrder.topDown),
          17);
    });
  });
}
