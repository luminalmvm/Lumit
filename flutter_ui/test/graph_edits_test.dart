// The graph's linked-pair arithmetic: what the other axis of a linked Scale
// takes when the drawn curve is written. Pure, so it is checked here rather
// than through the engine, as graph_maths_test.dart checks the evaluator.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/graph_edits.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

BridgeKeyframe key(
  int seconds,
  double value, {
  BridgeSideInterp interpIn = const BridgeSideInterp.linear(),
  BridgeSideInterp interpOut = const BridgeSideInterp.linear(),
}) =>
    BridgeKeyframe(
        time: BridgeRational(num: seconds, den: 1),
        value: value,
        interpIn: interpIn,
        interpOut: interpOut);

List<double> valuesOf(BridgeScalar scalar) =>
    [for (final k in (scalar as BridgeScalar_Keyframed).field0) k.value];

void main() {
  group('linkedPartnerScalar', () {
    const eased =
        BridgeSideInterp.bezier(BridgeBezierSide(speed: 40, influence: 0.3));
    final lead = BridgeScalar.keyframed([key(0, 100), key(1, 200)]);
    final partner = BridgeScalar.keyframed([key(0, 50), key(1, 100)]);
    // The lead rewritten: a key moved, a key eased.
    final next = BridgeScalar.keyframed(
        [key(0, 100, interpOut: eased), key(2, 300, interpIn: eased)]);

    test('the partner takes the lead\'s keys at the ratio the pair held', () {
      final got = linkedPartnerScalar(lead, partner, next);
      expect(valuesOf(got), [50.0, 150.0]);
      final keys = (got as BridgeScalar_Keyframed).field0;
      expect([for (final k in keys) k.time],
          [for (final k in (next as BridgeScalar_Keyframed).field0) k.time]);
      const halved =
          BridgeSideInterp.bezier(BridgeBezierSide(speed: 20, influence: 0.3));
      expect(keys.first.interpOut, halved,
          reason: 'speed is on the value axis, influence is not');
      expect(keys.last.interpIn, halved);
    });

    test('a key at nought does not decide the ratio', () {
      final fromNothing = BridgeScalar.keyframed([key(0, 0), key(1, 100)]);
      final half = BridgeScalar.keyframed([key(0, 0), key(1, 50)]);
      expect(valuesOf(linkedPartnerScalar(fromNothing, half, next)),
          [50.0, 150.0]);
    });

    test('a lead at nought everywhere has no ratio, so the partner matches it',
        () {
      expect(
          valuesOf(linkedPartnerScalar(const BridgeScalar.static_(0),
              const BridgeScalar.static_(50), next)),
          [100.0, 300.0]);
    });

    test('a static pair keyed through the graph keys both', () {
      expect(
          valuesOf(linkedPartnerScalar(const BridgeScalar.static_(100),
              const BridgeScalar.static_(50), next)),
          [50.0, 150.0]);
    });

    test('the last key deleted leaves the partner static at the ratio', () {
      expect(
          linkedPartnerScalar(lead, partner, const BridgeScalar.static_(200)),
          const BridgeScalar.static_(100));
    });
  });
}
