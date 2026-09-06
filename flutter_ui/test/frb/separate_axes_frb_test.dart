// Separate axes: a Position that comes apart into a row per axis, and
// goes back together without moving the picture.
//
// What is pinned here is the wiring, because the storage needed none — the axes
// were always separate scalar properties. So: the fold-out grows a row per axis
// and shrinks back; the graph editor aims one curve at a separated axis rather
// than the pair's two; Scale starts linked and draws one box; and the whole
// thing survives the trip through the engine as the read model reports it.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/graph_editor_frb.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/panels/transform_rows_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Separate axes (frb)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    LayerReference solid(dynamic p) {
      (p.comp as CompositionReference).addSolidLayer();
      return (p.comp as CompositionReference).getLayers().single;
    }

    BridgeLayerEntry entryOf(dynamic p) =>
        (p.comp as CompositionReference).getModel().layers.single;

    List<String> transformRowLabels(dynamic p) => layerFoldRows(
          entry: entryOf(p),
          open: {transformPath((entryOf(p).layer.internallayerId).toString())},
          hasAudio: false,
        ).whereType<FoldTransformRow>().map((r) => r.group.label).toList();

    testWidgets('a fresh layer is combined, combined, linked', (tester) async {
      final p = withComp();
      solid(p);
      final modes = entryOf(p).info.axisModes;
      expect(modes.anchor, BridgeAxisMode.combined);
      expect(modes.position, BridgeAxisMode.combined);
      expect(modes.scale, BridgeAxisMode.linked,
          reason: 'a scale that quietly stops being proportional is a mistake');
    });

    testWidgets('separating Position gives it a row per axis, and combining takes '
        'them back', (tester) async {
      final p = withComp();
      final layer = solid(p);

      expect(transformRowLabels(p), contains('Position'));
      expect(transformRowLabels(p), isNot(contains('Position x')));

      layer.setAxisMode(
          pair: BridgeTransformPair.position, mode: BridgeAxisMode.separated);
      final separated = transformRowLabels(p);
      expect(separated, isNot(contains('Position')));
      expect(separated, containsAll(<String>['Position x', 'Position y']));
      expect(separated, isNot(contains('Position z')),
          reason: 'a 2D layer draws no z row, separated or not');
      // The other pairs are untouched: the choice is per property, not per
      // layer.
      expect(separated, contains('Anchor point'));
      expect(separated, contains('Scale'));

      layer.setAxisMode(
          pair: BridgeTransformPair.position, mode: BridgeAxisMode.combined);
      expect(transformRowLabels(p), contains('Position'));
      expect(transformRowLabels(p), isNot(contains('Position x')));
    });

    testWidgets('a separated axis is one curve in the graph, where the pair was two',
        (tester) async {
      final p = withComp();
      final layer = solid(p);
      final id = layer.internallayerId.toString();

      final pairPath = transformGroupPath(
        id,
        transformGroups(threeD: false, modes: entryOf(p).info.axisModes)
            .firstWhere((g) => g.label == 'Position'),
      );
      expect(
        graphChannels(layers: [entryOf(p)], selected: [pairPath]).length,
        2,
        reason: 'a combined Position is its x and y strokes',
      );

      layer.setAxisMode(
          pair: BridgeTransformPair.position, mode: BridgeAxisMode.separated);
      final yPath = transformGroupPath(
        id,
        transformGroups(threeD: false, modes: entryOf(p).info.axisModes)
            .firstWhere((g) => g.label == 'Position y'),
      );
      final channels = graphChannels(layers: [entryOf(p)], selected: [yPath]);
      expect(channels.length, 1);
      expect(channels.single.prop, BridgeTransformProp.positionY);
    });

    testWidgets('Scale draws one box while it is linked, and two once unlinked', (tester) async {
      final p = withComp();
      final layer = solid(p);

      TransformGroup scaleRow() =>
          transformGroups(threeD: false, modes: entryOf(p).info.axisModes)
              .firstWhere((g) => g.label.startsWith('Scale'));

      expect(scaleRow().isLinked, isTrue);
      expect(scaleRow().axes.length, 2,
          reason: 'one box, but the stopwatch still covers both axes');

      layer.setAxisMode(
          pair: BridgeTransformPair.scale, mode: BridgeAxisMode.combined);
      expect(scaleRow().isLinked, isFalse);

      layer.setAxisMode(
          pair: BridgeTransformPair.scale, mode: BridgeAxisMode.separated);
      final rows = transformRowLabels(p);
      expect(rows, containsAll(<String>['Scale x', 'Scale y']));
    });

    /// A linked Scale is one curve as it is one box: the graph draws the lead
    /// axis, and an edit to it reaches the other at the ratio the pair holds.
    /// Two curves eased one at a time was the reported bug: easing x left y
    /// linear, and the picture stretched on the way.
    testWidgets(
        'a linked Scale is one curve, and an ease on it reaches both axes',
        (tester) async {
      final p = withComp();
      final layer = solid(p);
      final id = layer.internallayerId.toString();
      BridgeKeyframe key(int seconds, double value) => BridgeKeyframe(
            time: BridgeRational(num: seconds, den: 1),
            value: value,
            interpIn: const BridgeSideInterp.linear(),
            interpOut: const BridgeSideInterp.linear(),
          );
      layer.setTransforms(props: [
        BridgeTransformProp.scaleX,
        BridgeTransformProp.scaleY,
      ], values: [
        BridgeScalar.keyframed([key(0, 100), key(1, 200)]),
        BridgeScalar.keyframed([key(0, 50), key(1, 100)]),
      ]);
      final scalePath = transformGroupPath(
        id,
        transformGroups(threeD: false, modes: entryOf(p).info.axisModes)
            .firstWhere((g) => g.label == 'Scale'),
      );

      final channels =
          graphChannels(layers: [entryOf(p)], selected: [scalePath]);
      expect(channels.length, 1,
          reason: 'one box on the row, one curve on the graph');
      expect(channels.single.prop, BridgeTransformProp.scaleX);
      expect(channels.single.linkedPartner, BridgeTransformProp.scaleY);
      expect(channels.single.label, endsWith('Scale'),
          reason: 'no axis letter on a row that shows none');

      const eased =
          BridgeSideInterp.bezier(BridgeBezierSide(speed: 0, influence: 0.5));
      applyInterpToSelection(
        channels: channels,
        selectedKeys: {'${channels.single.id}#0', '${channels.single.id}#1'},
        side: eased,
      );

      final tf = layer.getTransform();
      final x = (tf.scaleX as BridgeScalar_Keyframed).field0;
      final y = (tf.scaleY as BridgeScalar_Keyframed).field0;
      expect(x.first.interpOut, eased);
      expect(y.first.interpOut, eased,
          reason: 'the ease reached the axis the graph does not draw');
      expect([for (final k in y) k.value], [50.0, 100.0],
          reason: 'the ratio held');
      expect([for (final k in y) k.time], [for (final k in x) k.time]);

      // The pair was one write, so one undo takes both back.
      p.state.project!.undo();
      expect(
          (layer.getTransform().scaleY as BridgeScalar_Keyframed)
              .field0
              .first
              .interpOut,
          const BridgeSideInterp.linear());

      // Unlinked, the pair is two curves again.
      layer.setAxisMode(
          pair: BridgeTransformPair.scale, mode: BridgeAxisMode.combined);
      expect(
          graphChannels(layers: [entryOf(p)], selected: [scalePath]).length, 2);
    });

    testWidgets('a 3D layer separates Position into three rows',
        (tester) async {
      final p = withComp();
      final layer = solid(p)
        ..setSwitch(switch_: BridgeLayerSwitch.threeD, on_: true);
      layer.setAxisMode(
          pair: BridgeTransformPair.position, mode: BridgeAxisMode.separated);
      expect(
        transformRowLabels(p),
        containsAll(<String>['Position x', 'Position y', 'Position z']),
      );
    });
  });
}
