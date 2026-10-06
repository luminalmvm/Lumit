// A key's two eases as four typed numbers (docs/07 §5.3): what is read off a
// key, and what writing them back does to each side.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/graph_maths.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

BridgeRational rat(int n, int d) => BridgeRational(num: n, den: d);

BridgeKeyframe key(
  int n,
  int d,
  double v, {
  BridgeSideInterp interpIn = const BridgeSideInterp.linear(),
  BridgeSideInterp interpOut = const BridgeSideInterp.linear(),
}) =>
    BridgeKeyframe(
        time: rat(n, d), value: v, interpIn: interpIn, interpOut: interpOut);

const BridgeSideInterp bezier40 =
    BridgeSideInterp.bezier(BridgeBezierSide(speed: 40, influence: 0.25));

void main() {
  /// A ramp of three keys, a second apart, rising 100 a second: the middle
  /// key's straight sides read at the chord, and its ends have one side each.
  final ramp = [key(0, 1, 0), key(1, 1, 100), key(2, 1, 200)];

  group('keyEaseOf', () {
    test('reads a straight side at its chord and a bezier at its own numbers',
        () {
      expect(
          keyEaseOf(ramp, 1),
          const KeyEase(
              inSpeed: 100,
              inInfluence: 1 / 3,
              outSpeed: 100,
              outInfluence: 1 / 3));
      final shaped = [ramp[0], key(1, 1, 100, interpOut: bezier40), ramp[2]];
      final ease = keyEaseOf(shaped, 1);
      expect(ease.outSpeed, 40);
      expect(ease.outInfluence, 0.25);
      expect(ease.inSpeed, 100, reason: 'the untouched side still reads');
    });
  });

  group('keyWithEase', () {
    test('a typed speed makes a bezier at that speed, keeping the reach', () {
      final next = keyWithEase(ramp, 1, const KeyEase(outSpeed: 0));
      expect(
          next.interpOut,
          const BridgeSideInterp.bezier(
              BridgeBezierSide(speed: 0, influence: 1 / 3)));
      expect(next.interpIn, const BridgeSideInterp.linear(),
          reason: 'the side not typed into is left exactly as it was');
      expect(next.time, ramp[1].time);
      expect(next.value, 100);
    });
  });
}
