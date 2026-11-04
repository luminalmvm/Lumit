// A keyed Colour (Fill's Colour) shows its keys on its Timeline lane, moves
// them there as one, and draws a curve per channel in the graph.
//
// Its four channels key together from one stopwatch, but the lane read only a
// one-number value, so a keyed Colour drew no diamonds at all.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart';
import 'package:lumit_flutter/panels/graph_channels.dart';
import 'package:lumit_flutter/panels/graph_edits.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  BridgeKeyframe key(int seconds, double value) => BridgeKeyframe(
        time: BridgeRational(num: seconds, den: 1),
        value: value,
        interpIn: const BridgeSideInterp.linear(),
        interpOut: const BridgeSideInterp.linear(),
      );

  testWidgets('a keyed Fill Colour shows, moves and graphs its keys',
      (tester) async {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Scene');
    comp.addSolidLayer();
    final layer = comp.getLayers().single;
    layer.addEffect(name: 'fill');
    BridgeScalar keyed(double a, double b) =>
        BridgeScalar.keyframed([key(0, a), key(1, b)]);
    final stack = layer.getEffects();
    stack.single.setValue(
      id: 'colour',
      value: BridgeEffectValue.colour(BridgeColour(
        r: keyed(1, 0),
        g: keyed(0, 1),
        b: keyed(0, 0),
        a: keyed(1, 1),
      )),
    );
    layer.setEffects(effects: stack);

    BridgeLayerEntry entry() => comp.getModel().layers.single;
    final id = layer.internallayerId.toString();
    final fx = entry().info.effects.single;
    FoldEffectParamRow row() => layerFoldRows(
          entry: entry(),
          open: {effectsPath(id), effectPath(id, fx.id.toString())},
          hasAudio: false,
        )
            .whereType<FoldEffectParamRow>()
            .firstWhere((r) => r.param.id == 'colour');

    expect(laneKeysOf(row()).length, 2, reason: 'the keys show on the lane');

    // Dragging the second diamond to 2 s moves it on every channel.
    expect(
        moveLaneKeys(
            entry: entry(),
            row: row(),
            times: {1: const BridgeRational(num: 2, den: 1)}),
        isTrue);
    final moved = layer.getEffects().single.getValue(id: 'colour')
        as BridgeEffectValue_Colour;
    for (final c in [
      moved.field0.r,
      moved.field0.g,
      moved.field0.b,
      moved.field0.a
    ]) {
      expect((c as BridgeScalar_Keyframed).field0.last.time,
          const BridgeRational(num: 2, den: 1));
    }

    // The graph draws one curve per channel and writes one back alone.
    final path = '${effectsPath(id)}/${fx.id}/colour';
    final channels = graphChannels(layers: [entry()], selected: [path]);
    expect(channels.map((c) => c.component), [0, 1, 2, 3]);
    commitChannelEdits({channels[1]: keyed(0.5, 0.5)});
    final written = layer.getEffects().single.getValue(id: 'colour')
        as BridgeEffectValue_Colour;
    expect(
        (written.field0.g as BridgeScalar_Keyframed).field0.first.value, 0.5);
    expect((written.field0.r as BridgeScalar_Keyframed).field0.first.value, 1.0,
        reason: 'the other channels kept');
  });

  /// Type fill's rows read as columns once its Direction is Columns: the
  /// rows are the same, only their names follow what they do.
  testWidgets('Type fill in Columns names its rows as columns', (tester) async {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Scene');
    comp.addSolidLayer();
    final layer = comp.getLayers().single;
    layer.addEffect(name: 'type_fill');
    String label(String id) =>
        instanceParameters(comp.getModel().layers.single.info.effects.single)
            .firstWhere((r) => r.id == id)
            .label;
    expect(label('rows'), 'Rows');
    final stack = layer.getEffects();
    stack.single
        .setValue(id: 'direction', value: const BridgeEffectValue.choice(1));
    layer.setEffects(effects: stack);
    expect(label('rows'), 'Columns');
    expect(label('row_shift'), 'Column shift');
    expect(label('grid_height'), 'Grid width');
  });
}
