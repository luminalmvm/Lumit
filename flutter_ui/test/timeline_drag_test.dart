// The maths both halves of the Timeline slide by while a layer is dragged.
// Pure, so it is tested without an engine or a widget tree — and it
// has to be right in one place only, which is the point of it being shared.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';

void main() {
  // Three layers: the middle one twirled open with two fold rows, so the
  // heights are not all the same and a shift cannot accidentally be right.
  const heights = [22.0, 66.0, 22.0];

  test('dragging down carries the block past what it passes', () {
    const drag = LayerDrag(0, 2);
    // The lifted block travels the height of both blocks it overtakes.
    expect(layerDragShift(heights, drag, 0), 66.0 + 22.0);
    // Each of those moves one lift's height the other way.
    expect(layerDragShift(heights, drag, 1), -22.0);
    expect(layerDragShift(heights, drag, 2), -22.0);
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
    test('a slot is taken at the midpoint of the block being passed', () {
      // Below layer 1 (66 high) sits layer 2 (22): half of it is 11.
      expect(layerDragTarget(heights, 1, 10), 1);
      expect(layerDragTarget(heights, 1, 12), 2);
      // Above layer 1 sits layer 0 (22): half is 11.
      expect(layerDragTarget(heights, 1, -10), 1);
      expect(layerDragTarget(heights, 1, -12), 0);
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
  });

  /// Which blocks a lazy half has to build. Pure for the same reason the drag
  /// maths is: both halves window against it, and they have to agree exactly
  /// or the table comes apart.
  group('the window a viewport builds', () {
    // A hundred even rows: 2200 tall against a 200 viewport.
    final tall = List<double>.filled(100, 22);

    test('a screenful either side of the rows in view', () {
      // Scrolled to 660: the band runs 460..1060, rows 20 through 48.
      final (first, last) = blockWindow(tall, 660, 200);
      expect(first, 20);
      expect(last, 49);
      expect(last - first, lessThan(30),
          reason: 'the cost follows the viewport, not the hundred rows');
    });
  });
}
