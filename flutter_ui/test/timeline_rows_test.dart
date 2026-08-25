// The small arithmetic the Timeline's rows and chrome are built from — the
// twirl's reach across a selection to begin with. Pure, so it is checked here
// rather than by clicking in a widget tree, exactly as timeline_drag_test.dart
// checks the row-height maths.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/icons/icons.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
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
      expect(wholePixelInset(scrollGutterWidth, scrollbarThickness), 2);
      expect(
          wholePixelInset(scrollGutterWidth, scrollbarThickness) +
              scrollbarThickness,
          lessThanOrEqualTo(scrollGutterWidth));
    });

    test('the inset is always a whole pixel, and never negative', () {
      for (final extent in [0.0, 4.0, 7.0, 11.0, 12.0, 18.0, 23.5]) {
        final inset = wholePixelInset(extent, scrollbarThickness);
        expect(inset, inset.roundToDouble(), reason: 'a soft edge otherwise');
        expect(inset, greaterThanOrEqualTo(0));
      }
    });
  });

  group('A switch glyph sits on the pixel grid (§6.20)', () {
    // The two row heights the density dial offers, and the cell they sit in.
    for (final row in [23.0, 22.0]) {
      test('a 16px glyph in a ${row.toInt()}px row starts whole', () {
        final top = wholePixelInset(row, iconSize);
        expect(top, top.roundToDouble(),
            reason: 'centring puts the odd row on 3.5, and the icons\' own '
                'half-pixel nudge then lands the strokes back on a boundary');
        expect(top, (row - iconSize) ~/ 2);
      });
    }

    test('the cell width already centred whole, and is left where it was', () {
      expect(wholePixelInset(switchCellWidth, iconSize), 3);
    });
  });

  group('The slider\'s two landscapes nudge the zoom (§6.5)', () {
    test('a step is a doubling either way', () {
      expect(zoomNudged(4, inward: true, maxZoom: 64), 8);
      expect(zoomNudged(4, inward: false, maxZoom: 64), 2);
      expect(zoomKeyStep, 2);
    });

    test('the ends of the slider are the ends of the nudge', () {
      // The whole composition, and no further out than that.
      expect(zoomNudged(1, inward: false, maxZoom: 64), 1);
      expect(zoomNudged(1.5, inward: false, maxZoom: 64), 1);
      expect(zoomNudged(48, inward: true, maxZoom: 64), 64);
      // A comp shorter than full zoom-in shows has nowhere to travel.
      expect(zoomNudged(1, inward: true, maxZoom: 1), 1);
    });
  });

  group('Lane keys travel with a bar being moved (§6.26)', () {
    test('a move carries them, an edge trim does not', () {
      expect(keyShiftOf(barDragPreview('a', BarGrab.move, 12), 'a'), 12);
      expect(keyShiftOf(barDragPreview('a', BarGrab.trimIn, 12), 'a'), 0);
      expect(keyShiftOf(barDragPreview('a', BarGrab.trimOut, 12), 'a'), 0);
    });

    test('another layer\'s drag leaves this lane alone', () {
      expect(keyShiftOf(barDragPreview('b', BarGrab.move, 12), 'a'), 0);
    });

    test('nothing in flight is no shift', () {
      expect(keyShiftOf(null, 'a'), 0);
    });
  });

  group('The Animated filter keeps what is keyed, and its way in (6.43)', () {
    /// A row that carries one keyframe, at [frame] seconds' worth of nothing —
    /// the time never matters here, only that the row has a key at all.
    FoldRetimeRow keyed(int depth) => FoldRetimeRow(
          BridgeScalar.keyframed([
            const BridgeKeyframe(
              time: BridgeRational(num: 0, den: 1),
              value: 0,
              interpIn: BridgeSideInterp.linear(),
              interpOut: BridgeSideInterp.linear(),
            ),
          ]),
          depth: depth,
        );

    /// A row that can never be keyed, for the depth it stands at.
    FoldVolumeRow still(int depth) => FoldVolumeRow(depth: depth);

    FoldGroupRow heading(String path, int depth) =>
        FoldGroupRow(path: path, label: path, open: true, depth: depth);

    test('a heading with nothing keyed under it goes with its contents', () {
      final rows = [heading('transform', 1), still(2), still(2)];
      expect(animatedFoldRows(rows), isEmpty);
    });

    test('a heading stays when one row beneath it is keyed', () {
      final rows = [heading('transform', 1), still(2), keyed(2)];
      final kept = animatedFoldRows(rows);
      expect(kept.length, 2);
      expect(kept.first, rows.first, reason: 'the heading leads down to it');
      expect(kept.last, rows.last);
    });

    test('an effect keeps its own name and the Effects heading above it', () {
      final rows = [
        heading('effects', 1),
        heading('effects/glow', 2),
        keyed(3),
        heading('effects/blur', 2),
        still(3),
      ];
      expect(animatedFoldRows(rows), [rows[0], rows[1], rows[2]],
          reason: 'the effect with nothing keyed goes, headings and all');
    });

    test('a sibling heading that qualifies does not save the one that does not',
        () {
      final rows = [
        heading('transform', 1),
        still(2),
        heading('effects', 1),
        keyed(2),
      ];
      expect(animatedFoldRows(rows), [rows[2], rows[3]]);
    });

    test('nothing keyed anywhere is no rows at all', () {
      expect(animatedFoldRows([still(1), still(2)]), isEmpty);
      expect(animatedFoldRows(const []), isEmpty);
    });
  });
}
