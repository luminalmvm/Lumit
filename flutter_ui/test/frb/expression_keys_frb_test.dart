// Keys under an expression: a key command on them keeps the expression.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/graph_channels.dart';
import 'package:lumit_flutter/panels/graph_edits.dart';
import 'package:lumit_flutter/panels/graph_maths.dart' show easyEase;
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
    expect(channels.single.ownKeyIndex(0), 0,
        reason: 'a diamond picked on the layer lane finds its key');
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
