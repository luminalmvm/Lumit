// The small arithmetic the Timeline's rows and chrome are built from — the
// twirl's reach across a selection to begin with. Pure, so it is checked here
// rather than by clicking in a widget tree, exactly as timeline_drag_test.dart
// checks the row-height maths.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
import 'package:lumit_flutter/panels/transform_rows_frb.dart';
import 'package:lumit_flutter/icons/icons.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart' show BridgePortType;
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/timeline_columns.dart';
import 'package:uuid/uuid.dart';

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

  /// **Animation ▸ Reveal properties with keyframes / with animation / all
  /// modified properties** (K-684). Three rules over one fold-out, each a
  /// superset of the one before it: the diamonds, then the rows that move
  /// without diamonds, then everything a fresh layer would not carry.
  group('The three Reveal rules widen one at a time (K-684)', () {
    const centre = 960.0, middle = 540.0;
    BridgeScalar st(double v) => BridgeScalar.static_(v);

    /// A transform nobody has touched, in a 1920×1080 comp.
    BridgeTransform fresh({
      BridgeScalar? opacity,
      BridgeScalar? rotation,
      BridgeScalar? scale,
    }) =>
        BridgeTransform(
          anchorX: st(0),
          anchorY: st(0),
          positionX: st(centre),
          positionY: st(middle),
          positionZ: st(0),
          scaleX: scale ?? st(100),
          scaleY: scale ?? st(100),
          rotation: rotation ?? st(0),
          rotationX: st(0),
          rotationY: st(0),
          opacity: opacity ?? st(100),
        );

    const oneKey = BridgeScalar.keyframed([
      BridgeKeyframe(
        time: BridgeRational(num: 0, den: 1),
        value: 0,
        interpIn: BridgeSideInterp.linear(),
        interpOut: BridgeSideInterp.linear(),
      ),
    ]);

    FoldTransformRow row(String label, BridgeTransformProp prop,
            BridgeTransform transform) =>
        FoldTransformRow(
            TransformGroup(label, [TransformAxis(prop)]), transform,
            depth: 2);

    // Position, untouched: the one row that is neither animated nor changed,
    // and the one that needs the comp's size to know it.
    final still_ = row('Position', BridgeTransformProp.positionX, fresh());
    // Opacity, keyframed.
    final keyed_ =
        row('Opacity', BridgeTransformProp.opacity, fresh(opacity: oneKey));
    // Rotation, driven by an expression: no diamonds, and a different number
    // at every frame all the same.
    final expressed = row('Rotation', BridgeTransformProp.rotation,
        fresh(rotation: const BridgeScalar.expression('time * 10')));
    // Scale, typed to something else and left there.
    final changed = row('Scale', BridgeTransformProp.scaleX, fresh(scale: st(50)));

    const layerId = 'layer-a';
    final fxId = UuidValue.fromString(const Uuid().v4());
    final fxHeading = FoldGroupRow(
        path: '$layerId/effects/$fxId', label: 'Glow', open: true, depth: 2);
    const param = BridgeParamInfo(
        id: 'intensity',
        label: 'Intensity',
        kind: BridgeParamKind.float(
            default_: 1, sliderMin: 0, sliderMax: 10),
        unit: BridgeUnit.raw);
    final info = BridgeEffectInstanceInfo(
        id: fxId,
        name: 'lumit.glow',
        enabled: true,
        values: const [],
        linkedPairs: const [],
        derivedParams: const []);
    // A parameter at its default, with a wire from the node graph on it
    // (K-471): nothing has been typed into it and it is not the same at every
    // frame either.
    final driven = FoldEffectParamRow(
        info, param, const BridgeEffectValue.float(BridgeScalar.static_(1)),
        depth: 3,
        driven: (
          driver: 'Wobble',
          type: BridgePortType.number,
          noStream: false
        ));

    final rows = [still_, keyed_, expressed, changed, fxHeading, driven];

    List<LayerFoldRow> kept(RevealFilter filter) => revealFoldRows(rows, filter,
        compWidth: 1920, compHeight: 1080);

    test('with keyframes keeps the diamonds alone', () {
      expect(kept(RevealFilter.keyframed), [keyed_],
          reason: 'the expression, the wire and the typed value have no keys');
    });

    test('with animation adds the expression and the wire', () {
      expect(kept(RevealFilter.animated), [keyed_, expressed, fxHeading, driven],
          reason: 'the heading leads down to the driven parameter');
    });

    test('all modified adds the value somebody typed', () {
      expect(kept(RevealFilter.modified),
          [keyed_, expressed, changed, fxHeading, driven]);
    });

    test('an unmoved Position is modified by nothing', () {
      for (final filter in RevealFilter.values) {
        expect(kept(filter), isNot(contains(still_)),
            reason: 'it sits where a fresh layer puts it');
      }
    });

    test('a moved Position is only modified with the comp size to hand', () {
      final movedRow = FoldTransformRow(
          const TransformGroup(
              'Position', [TransformAxis(BridgeTransformProp.positionX)]),
          BridgeTransform(
            anchorX: st(0),
            anchorY: st(0),
            positionX: st(100),
            positionY: st(middle),
            positionZ: st(0),
            scaleX: st(100),
            scaleY: st(100),
            rotation: st(0),
            rotationX: st(0),
            rotationY: st(0),
            opacity: st(100),
          ),
          depth: 2);
      expect(
          revealFoldRows([movedRow], RevealFilter.modified,
              compWidth: 1920, compHeight: 1080),
          [movedRow]);
      // No comp size: Position is exempt rather than reported as moved, the
      // same answer the engine gives the Anchor.
      expect(revealFoldRows([movedRow], RevealFilter.modified), isEmpty);
    });

    test('an effect nobody touched still shows its name under the widest rule',
        () {
      final untouched = FoldEffectParamRow(
          info, param, const BridgeEffectValue.float(BridgeScalar.static_(1)),
          depth: 3);
      expect(
          revealFoldRows([fxHeading, untouched], RevealFilter.modified,
              compWidth: 1920, compHeight: 1080),
          [fxHeading],
          reason: 'applying an effect is a modification; its default is not');
      expect(
          revealFoldRows([fxHeading, untouched], RevealFilter.animated),
          isEmpty,
          reason: 'and nothing about it is animated');
    });
  });
}
