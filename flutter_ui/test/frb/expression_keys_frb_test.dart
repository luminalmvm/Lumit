// Keys under an expression, on the Timeline: the lane draws them and the row
// reads the number the render does.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/graph_channels.dart';
import 'package:lumit_flutter/panels/graph_edits.dart';
import 'package:lumit_flutter/panels/graph_maths.dart' show easyEase;
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

import 'frb_test_support.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(initEngineForTests);

  BridgeKeyframe key(int seconds, double value) => BridgeKeyframe(
        time: BridgeRational(num: seconds, den: 1),
        value: value,
        interpIn: const BridgeSideInterp.linear(),
        interpOut: const BridgeSideInterp.linear(),
      );

  final looped = BridgeScalar.expression('loopOut("cycle")',
      BridgeExpressionLanguage.javaScript, [key(0, 0), key(2, 2)]);

  test('a looped Retime draws its keys on the lane', () {
    expect(laneKeysOf(FoldRetimeRow(looped, depth: 1)), hasLength(2));
  });

  test('a keyed Retime with an expression shows it on a line beneath', () {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Scene');
    final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
    comp.addFootageLayer(footage: footage, asSequence: false);
    final layer = comp.getLayers().single;
    p.uiState.setSelectedComp(comp);
    layer.toggleRetimeProperty();
    layer.setRetimeProperty(value: BridgeScalar.keyframed([key(0, 0), key(2, 2)]));
    layer.setRetimeProperty(
        value: BridgeScalar.expression(
            'loopOut("cycle")', BridgeExpressionLanguage.javaScript));
    p.uiState.model.refresh();
    final rows = layerFoldRows(
      entry: p.uiState.model.layers.single,
      open: everyFoldPath,
      hasAudio: false,
    );
    final at = rows.indexWhere((r) => r is FoldRetimeRow);
    expect(at, isNot(-1));
    expect(rows[at + 1], isA<FoldExpressionRow>());
    expect(laneKeysOf(rows[at]), hasLength(2), reason: 'keys kept under it');
  });

  test('a looped Retime reads the moment the loop is at, not -1', () {
    double at(int seconds) =>
        sampleScalar(scalar: looped, time: BridgeRational(num: seconds, den: 1));
    expect(at(1), closeTo(1, 1e-9));
    expect(at(3), closeTo(1, 1e-9), reason: 'one second into the second lap');
  });
  test('F9 eases keys under an expression and keeps the expression', () {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Scene');
    final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
    comp.addFootageLayer(footage: footage, asSequence: false);
    final layer = comp.getLayers().single;
    p.uiState.setSelectedComp(comp);
    layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.keyframed([key(0, 0), key(2, 100)]));
    layer.setTransform(
        prop: BridgeTransformProp.opacity,
        value: BridgeScalar.expression(
            'loopOut()', BridgeExpressionLanguage.javaScript));
    p.uiState.model.refresh();
    final id = layer.internallayerId.toString();
    final channels = graphChannels(
        layers: p.uiState.model.layers, selected: ['$id/transform/opacity']);
    expect(channels.single.keys, hasLength(2));
    applyInterpToSelection(
      channels: channels,
      selectedKeys: {'${channels.single.id}#0'},
      side: easyEase,
    );
    final now = layer.getTransform().opacity;
    expect(now, isA<BridgeScalar_Expression>());
    final keys = (now as BridgeScalar_Expression).field2!;
    expect(keys, hasLength(2));
    expect(keys[0].interpOut, easyEase);
  });
}
