// Keys under an expression, on the Timeline: the lane draws them and the row
// reads the number the render does.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

import 'frb_test_support.dart';

void main() {
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

  test('a looped Retime reads the moment the loop is at, not -1', () {
    double at(int seconds) =>
        sampleScalar(scalar: looped, time: BridgeRational(num: seconds, den: 1));
    expect(at(1), closeTo(1, 1e-9));
    expect(at(3), closeTo(1, 1e-9), reason: 'one second into the second lap');
  });
}
