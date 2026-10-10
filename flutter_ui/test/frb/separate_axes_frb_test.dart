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

    /// The same pair on a Transform effect, whose Scale is two rows with a
    /// chain between them. Chained, either row is the one curve, and an ease
    /// on it reaches the half the graph does not draw.
    testWidgets(
        'a chained effect Scale is one curve, and an ease on it reaches both rows',
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
      layer.addEffect(name: 'transform');
      final staged = layer.getEffects();
      final fxId = staged.single.id();
      staged.single.setPairLinked(stem: 'scale', linked: true);
      staged.single.setValue(
          id: 'scale_x',
          value: BridgeEffectValue.float(
              BridgeScalar.keyframed([key(0, 100), key(1, 200)])));
      staged.single.setValue(
          id: 'scale_y',
          value: BridgeEffectValue.float(
              BridgeScalar.keyframed([key(0, 50), key(1, 100)])));
      layer.setEffects(effects: staged);

      final x = '$id/effects/$fxId/scale_x';
      final y = '$id/effects/$fxId/scale_y';
      final channels = graphChannels(layers: [entryOf(p)], selected: [x, y]);
      expect(channels.length, 1, reason: 'both rows are the one curve');
      expect(channels.single.param?.id, 'scale_x');
      expect(channels.single.linkedParam?.id, 'scale_y');
      expect(
          graphChannels(layers: [entryOf(p)], selected: [y]).single.param?.id,
          'scale_x',
          reason: 'the y row alone still opens the pair');

      const eased =
          BridgeSideInterp.bezier(BridgeBezierSide(speed: 0, influence: 0.5));
      applyInterpToSelection(
        channels: channels,
        selectedKeys: {'${channels.single.id}#0', '${channels.single.id}#1'},
        side: eased,
      );

      List<BridgeKeyframe> keysOfRow(String row) {
        for (final v in entryOf(p).info.effects.single.values) {
          if (v.id == row) {
            final scalar = (v.value as BridgeEffectValue_Float).field0;
            return (scalar as BridgeScalar_Keyframed).field0;
          }
        }
        return const [];
      }

      expect(keysOfRow('scale_x').first.interpOut, eased);
      expect(keysOfRow('scale_y').first.interpOut, eased,
          reason: 'the ease reached the row the graph does not draw');
      expect([for (final k in keysOfRow('scale_y')) k.value], [50.0, 100.0],
          reason: 'the ratio held');

      // Unchained, the pair is two curves again.
      final again = layer.getEffects();
      again.single.setPairLinked(stem: 'scale', linked: false);
      layer.setEffects(effects: again);
      expect(graphChannels(layers: [entryOf(p)], selected: [x, y]).length, 2);
    });
  });
}
