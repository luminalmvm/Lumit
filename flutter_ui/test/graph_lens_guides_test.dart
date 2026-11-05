// The turning points the stacked graphs ring: where the speed peaks (the
// value's steepest point), and none on a straight run.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/graph_channels.dart';
import 'package:lumit_flutter/panels/graph_lens_guides.dart';
import 'package:lumit_flutter/panels/graph_maths.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

BridgeKeyframe key(int seconds, double v, BridgeSideInterp side) =>
    BridgeKeyframe(
        time: BridgeRational(num: seconds, den: 1),
        value: v,
        interpIn: side,
        interpOut: side);

void main() {
  accelerationTests();
  constantAccelerationTests();
  const ease =
      BridgeSideInterp.bezier(BridgeBezierSide(speed: 0, influence: 1 / 3));
  const linear = BridgeSideInterp.linear();

  test('an easy ease turns once, half way: the fastest moment', () {
    final keys = [key(0, 0, ease), key(2, 100, ease)];
    final speed = turningPointsOf(keys, GraphLens.speed);
    expect(speed, hasLength(1));
    expect(speed.single, closeTo(1, 0.05));
    // The value graph rings the same moment: its steepest point.
    final value = turningPointsOf(keys, GraphLens.value);
    expect(value, hasLength(1));
    expect(value.single, closeTo(1, 0.05));
  });

  test('a straight run has no turning point', () {
    final keys = [key(0, 0, linear), key(2, 100, linear)];
    for (final lens in GraphLens.values) {
      expect(turningPointsOf(keys, lens), isEmpty, reason: '$lens');
    }
  });

  test('a peak of the value is a turning point of the value', () {
    final keys = [key(0, 0, ease), key(1, 100, ease), key(2, 0, ease)];
    // The peak sits on the middle key, which is drawn already; the two
    // steepest moments are inside the spans.
    final value = turningPointsOf(keys, GraphLens.value);
    expect(value, hasLength(2));
    expect(value[0], closeTo(0.5, 0.05));
    expect(value[1], closeTo(1.5, 0.05));
  });
}

/// A typed acceleration is met at the key, whichever way it points: by the
/// influence where that can reach it, by the side's speed where it cannot.
void accelerationTests() {
  const ease =
      BridgeSideInterp.bezier(BridgeBezierSide(speed: 0, influence: 1 / 3));

  BridgeKeyframe withSide(BridgeKeyframe k, BridgeSideInterp side,
          {required bool isOut}) =>
      BridgeKeyframe(
          time: k.time,
          value: k.value,
          interpIn: isOut ? k.interpIn : side,
          interpOut: isOut ? side : k.interpOut);

  for (final isOut in [true, false]) {
    for (final target in [3.0, 40.0, 500.0, -3.0, -200.0]) {
      test('${isOut ? 'out' : 'in'} side, $target /s²', () {
        final keys = [key(0, 0, ease), key(1, 100, ease), key(2, 0, ease)];
        final side =
            sideForAcceleration(keys, 1, isOut: isOut, target: target);
        expect(side, isNotNull);
        final shaped = [
          keys[0],
          withSide(keys[1], side!, isOut: isOut),
          keys[2],
        ];
        expect(sideAccelerationAtKey(shaped, 1, isOut: isOut),
            closeTo(target, 1e-6 * (target.abs() + 1)));
      });
    }
  }

  test('no neighbour, no side', () {
    final keys = [key(0, 0, ease), key(1, 100, ease)];
    expect(sideForAcceleration(keys, 1, isOut: true, target: 3), isNull);
  });
}

/// A stretch of constant acceleration from rest: the parabola, held exactly.
void constantAccelerationTests() {
  const ease =
      BridgeSideInterp.bezier(BridgeBezierSide(speed: 0, influence: 1 / 3));

  test('the three numbers, two given', () {
    final a = constantAcceleration(0, acceleration: 20, endValue: 100)!;
    expect(a.seconds, closeTo(3.1623, 1e-4));
    final b = constantAcceleration(0, acceleration: 20, seconds: 2)!;
    expect(b.endValue, closeTo(40, 1e-9));
    final c = constantAcceleration(0, endValue: 90, seconds: 3)!;
    expect(c.acceleration, closeTo(20, 1e-9));
    expect(constantAcceleration(0, acceleration: -20, endValue: 100), isNull,
        reason: 'slowing down never reaches a higher value from rest');
    expect(constantAcceleration(0, acceleration: 20, seconds: 0), isNull);
  });

  test('one key grows a second, and the acceleration is the same throughout',
      () {
    final keys = [key(0, 0, ease)];
    final shaped = keysWithConstantAcceleration(keys, 0,
        acceleration: 20,
        endTime: BridgeRational(num: 3, den: 1),
        endValue: 90)!;
    expect(shaped, hasLength(2));
    for (var t = 0.1; t < 3; t += 0.29) {
      expect(evaluateKeysAcceleration(shaped, t), closeTo(20, 1e-6));
      expect(evaluateKeys(shaped, t), closeTo(10 * t * t, 1e-6));
    }
    expect(evaluateKeysSpeed(shaped, 1e-6), closeTo(0, 1e-3),
        reason: 'it starts at rest');
  });

  test('the next key is moved, the one after it kept', () {
    final keys = [key(0, 0, ease), key(1, 50, ease), key(5, 0, ease)];
    final shaped = keysWithConstantAcceleration(keys, 0,
        acceleration: 20,
        endTime: BridgeRational(num: 2, den: 1),
        endValue: 40)!;
    expect(shaped, hasLength(3));
    expect(shaped[1].value, 40);
    expect(shaped[2].value, 0);
    expect(
        keysWithConstantAcceleration(keys, 0,
            acceleration: 20,
            endTime: BridgeRational(num: 6, den: 1),
            endValue: 360),
        isNull,
        reason: 'it may not jump the key after the next');
  });
}
