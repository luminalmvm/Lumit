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

    test('an end key has one side', () {
      final first = keyEaseOf(ramp, 0);
      expect(first.hasIn, isFalse);
      expect(first.hasOut, isTrue);
      final last = keyEaseOf(ramp, 2);
      expect(last.hasIn, isTrue);
      expect(last.hasOut, isFalse);
      expect(keyEaseOf([ramp[0]], 0).isEmpty, isTrue,
          reason: 'a lone key has no span on either side');
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

    test('a typed influence keeps the speed the side reads at', () {
      final next = keyWithEase(ramp, 1, const KeyEase(inInfluence: 0.8));
      expect(
          next.interpIn,
          const BridgeSideInterp.bezier(
              BridgeBezierSide(speed: 100, influence: 0.8)));
    });

    test('both numbers on one side land together, clamped to a legal reach',
        () {
      final next =
          keyWithEase(ramp, 1, const KeyEase(outSpeed: 40, outInfluence: 0));
      final side = next.interpOut as BridgeSideInterp_Bezier;
      expect(side.field0.speed, 40);
      expect(side.field0.influence, minTangentReach,
          reason: 'never quite vertical');
    });

    test('an automatic side typed into becomes free; one left alone stays', () {
      const auto = BridgeSideInterp.auto(
          BridgeAutoSide(clamped: true, speed: 7, influence: 0.5));
      final keys = [
        ramp[0],
        key(1, 1, 100, interpIn: auto, interpOut: auto),
        ramp[2],
      ];
      final next = keyWithEase(keys, 1, const KeyEase(outSpeed: 12));
      expect(
          next.interpOut,
          const BridgeSideInterp.bezier(
              BridgeBezierSide(speed: 12, influence: 0.5)),
          reason: 'its own reach, the typed speed, and free from here');
      expect(next.interpIn, auto);
    });

    test('nothing typed changes nothing', () {
      final next = keysWithEase(ramp, 1, const KeyEase());
      expect(next[1].interpIn, ramp[1].interpIn);
      expect(next[1].interpOut, ramp[1].interpOut);
      expect(next[0], ramp[0]);
      expect(next[2], ramp[2]);
    });
  });

  group('KeyEase', () {
    test('merge lays the later numbers over the earlier', () {
      const a = KeyEase(inSpeed: 1, outSpeed: 2);
      const b = KeyEase(outSpeed: 3, outInfluence: 0.4);
      expect(a.merge(b),
          const KeyEase(inSpeed: 1, outSpeed: 3, outInfluence: 0.4));
    });

    test('side keeps one side only', () {
      const both =
          KeyEase(inSpeed: 1, inInfluence: 0.1, outSpeed: 2, outInfluence: 0.2);
      expect(both.side(isOut: true),
          const KeyEase(outSpeed: 2, outInfluence: 0.2));
      expect(
          both.side(isOut: false), const KeyEase(inSpeed: 1, inInfluence: 0.1));
    });
  });
}
