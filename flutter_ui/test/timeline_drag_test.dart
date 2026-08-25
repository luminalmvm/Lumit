// The maths both halves of the Timeline slide by while a layer is dragged
// (K-208). Pure, so it is tested without an engine or a widget tree — and it
// has to be right in one place only, which is the point of it being shared.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';

void main() {
  // Three layers: the middle one twirled open with two fold rows, so the
  // heights are not all the same and a shift cannot accidentally be right.
  const heights = [22.0, 66.0, 22.0];

  test('nothing moves when nothing is being dragged', () {
    for (var i = 0; i < heights.length; i++) {
      expect(layerDragShift(heights, null, i), 0);
      expect(layerDragShift(heights, const LayerDrag(1, 1), i), 0);
    }
  });

  test('dragging down carries the block past what it passes', () {
    const drag = LayerDrag(0, 2);
    // The lifted block travels the height of both blocks it overtakes.
    expect(layerDragShift(heights, drag, 0), 66.0 + 22.0);
    // Each of those moves one lift's height the other way.
    expect(layerDragShift(heights, drag, 1), -22.0);
    expect(layerDragShift(heights, drag, 2), -22.0);
  });

  test('dragging up is the same in reverse', () {
    const drag = LayerDrag(2, 0);
    expect(layerDragShift(heights, drag, 2), -(22.0 + 66.0));
    expect(layerDragShift(heights, drag, 0), 22.0);
    expect(layerDragShift(heights, drag, 1), 22.0);
  });

  test('a block outside the moved span stays put', () {
    const four = [22.0, 22.0, 22.0, 22.0];
    const drag = LayerDrag(0, 1);
    expect(layerDragShift(four, drag, 2), 0);
    expect(layerDragShift(four, drag, 3), 0);
  });

  test('an index that has gone away is left alone', () {
    // The stack can shrink under a drag — a delete, a filter, a search.
    expect(layerDragShift(heights, const LayerDrag(0, 9), 0), 0);
    expect(layerDragShift(heights, const LayerDrag(9, 0), 0), 0);
    expect(layerDragShift(heights, const LayerDrag(0, 2), 7), 0);
  });

  // The drag targeting: travel against the *original* heights. The old scheme
  // asked which row the pointer was over, but the rows are slid by the drag
  // itself, so each answer moved the rows and changed the next one.
  group('drag targeting', () {
    test('no travel is no move, so a drag put back where it began is a no-op',
        () {
      expect(layerDragTarget(heights, 1, 0), 1);
      expect(layerDragTarget(heights, 1, 4), 1);
      expect(layerDragTarget(heights, 1, -4), 1);
    });

    test('a slot is taken at the midpoint of the block being passed', () {
      // Below layer 1 (66 high) sits layer 2 (22): half of it is 11.
      expect(layerDragTarget(heights, 1, 10), 1);
      expect(layerDragTarget(heights, 1, 12), 2);
      // Above layer 1 sits layer 0 (22): half is 11.
      expect(layerDragTarget(heights, 1, -10), 1);
      expect(layerDragTarget(heights, 1, -12), 0);
    });

    test('the target is monotone in travel, so it cannot ping-pong', () {
      var last = 0;
      for (var travel = 0.0; travel <= 200; travel += 1) {
        final to = layerDragTarget(heights, 0, travel);
        expect(to, greaterThanOrEqualTo(last),
            reason: 'target went backwards at travel $travel');
        last = to;
      }
    });

    test('travel past the ends stops at the ends', () {
      expect(layerDragTarget(heights, 0, 10000), heights.length - 1);
      expect(layerDragTarget(heights, 2, -10000), 0);
    });

    test('a from-index that has gone away is left alone', () {
      expect(layerDragTarget(heights, 9, 100), 9);
      expect(layerDragTarget(heights, -1, 100), -1);
    });
  });

  group('where a Project-panel drop lands', () {
    test('the top half of a block goes above it, the bottom half below', () {
      expect(layerDropSlot(heights, 0), 0);
      expect(layerDropSlot(heights, 10), 0, reason: 'top half of block 0');
      expect(layerDropSlot(heights, 12), 1, reason: 'bottom half of block 0');
      expect(layerDropSlot(heights, 40), 1, reason: 'top half of block 1');
      expect(layerDropSlot(heights, 60), 2, reason: 'bottom half of block 1');
    });

    test('a drop past the last block lands at the bottom of the stack', () {
      expect(layerDropSlot(heights, 10000), heights.length);
    });

    test('an empty stack takes the drop at nought', () {
      expect(layerDropSlot(const [], 500), 0);
    });
  });
}
