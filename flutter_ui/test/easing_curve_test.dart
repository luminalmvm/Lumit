// The easing curve's conversion, against hand-computed values and against the
// engine's own easy-ease constant (crates/lumit-core/src/anim.rs). The mapping
// under test is the one derived in docs/impl/keyframe-eval.md §1: a normalised
// shape becomes a (speed, influence) pair per span, and speed carries the
// span's chord slope while influence does not.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/easing_curve.dart';
import 'package:lumit_flutter/panels/graph_maths.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

/// The (speed, influence) of a side, or null where it is not a bezier.
({double speed, double influence})? bez(BridgeSideInterp side) =>
    switch (side) {
      BridgeSideInterp_Bezier(:final field0) => (
          speed: field0.speed,
          influence: field0.influence
        ),
      _ => null,
    };

void main() {
  group('EasingCurve', () {
    test('clamps x into the span and y into the editor view', () {
      // x is time and must stay in the span for the curve to be x-monotone
      // (keyframe-eval.md §1). y may leave the box — overshoot is the point of
      // it — but only as far as the editor draws, or the handle lands where no
      // pointer can reach it back.
      final past = EasingCurve(-1, -2, 5, 3);
      expect(past.x1, minTangentReach);
      expect(past.x2, 1 - minTangentReach);
      expect(past.y1, -easingHandleReach);
      expect(past.y2, 1 + easingHandleReach);
    });

    test('the easy-ease shape converts to the engine easy-ease constant', () {
      // The first preset is drawn as F9's ease: flat at both ends, influence
      // one third. Converting it must land exactly on `easyEase`, whatever the
      // span — flat means speed 0, and speed 0 scales to speed 0.
      final sides = easingPresets.first.curve.sidesFor(42);
      expect(bez(sides.out)!.speed, closeTo(0, 1e-12));
      expect(bez(sides.out)!.influence, closeTo(1 / 3, 1e-12));
      expect(bez(sides.inTo)!.speed, closeTo(0, 1e-12));
      expect(bez(sides.inTo)!.influence, closeTo(1 / 3, 1e-12));
    });

    test('influence ignores the chord, speed scales with it', () {
      // The whole reason the conversion is per span: the same drawn shape is a
      // different stored speed on a span that moves further in the same time,
      // and that is what makes it *look* the same on both.
      final curve = EasingCurve(0.25, 0.5, 0.75, 0.5);
      final slow = curve.sidesFor(1);
      final fast = curve.sidesFor(10);
      expect(bez(slow.out)!.influence, bez(fast.out)!.influence);
      expect(bez(slow.inTo)!.influence, bez(fast.inTo)!.influence);
      expect(bez(fast.out)!.speed, closeTo(bez(slow.out)!.speed * 10, 1e-12));
      expect(bez(fast.inTo)!.speed, closeTo(bez(slow.inTo)!.speed * 10, 1e-12));
    });
  });

  group('the drawn shape', () {
    test('advances in time across every preset', () {
      // x-monotonicity, checked on the shapes actually shipped: time may never
      // run backwards inside a span, or the span stops being solvable.
      for (final preset in easingPresets) {
        var last = 0.0;
        for (var i = 1; i <= 64; i++) {
          final x = preset.curve.xAt(i / 64);
          expect(x, greaterThan(last - 1e-12), reason: preset.id);
          last = x;
        }
      }
    });
  });
}
