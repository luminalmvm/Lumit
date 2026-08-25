// The small arithmetic the Timeline's rows and chrome are built from — the
// twirl's reach across a selection to begin with. Pure, so it is checked here
// rather than by clicking in a widget tree, exactly as timeline_drag_test.dart
// checks the row-height maths.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/state/timeline_columns.dart';

void main() {
  group('A twirl on a selected row moves the selection with it (§6.4)', () {
    test('an unselected row twirls alone', () {
      expect(rowsTwirledWith('layer-a', {'layer-b', 'layer-c'}), {'layer-a'});
    });

    test('a selected row takes every other selected row', () {
      expect(rowsTwirledWith('layer-a', {'layer-a', 'layer-b'}),
          {'layer-a', 'layer-b'});
    });

    test('property rows and layer rows are the same rule', () {
      expect(
        rowsTwirledWith('layer-a/transform', {
          'layer-a/transform',
          'layer-b/effects/fx-1',
        }),
        {'layer-a/transform', 'layer-b/effects/fx-1'},
      );
    });

    test('nothing selected leaves the clicked row on its own', () {
      expect(rowsTwirledWith('layer-a', const {}), {'layer-a'});
    });
  });

  group('Every scrollbar thumb is the same 7 (§6.15)', () {
    test('the vertical thumb fits its gutter with room either side', () {
      expect(scrollbarThickness, 7);
      expect(scrollbarThickness, lessThan(scrollGutterWidth));
      // The 12px gutter: 2 either side of the 7, floored rather than 2.5.
      expect(scrollbarInset(scrollGutterWidth), 2);
      expect(scrollbarInset(scrollGutterWidth) + scrollbarThickness,
          lessThanOrEqualTo(scrollGutterWidth));
    });

    test('the inset is always a whole pixel, and never negative', () {
      for (final extent in [0.0, 4.0, 7.0, 11.0, 12.0, 18.0, 23.5]) {
        final inset = scrollbarInset(extent);
        expect(inset, inset.roundToDouble(), reason: 'a soft edge otherwise');
        expect(inset, greaterThanOrEqualTo(0));
      }
    });
  });
}
