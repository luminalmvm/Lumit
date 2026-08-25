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

    test('y inside the view is left exactly alone', () {
      final curve = EasingCurve(0.3, -0.2, 0.7, 1.2);
      expect(curve.y1, closeTo(-0.2, 1e-12));
      expect(curve.y2, closeTo(1.2, 1e-12));
    });

    test('every preset sits inside the reach it is drawn in', () {
      for (final preset in easingPresets) {
        for (final y in [preset.curve.y1, preset.curve.y2]) {
          expect(y, greaterThanOrEqualTo(-easingHandleReach),
              reason: preset.id);
          expect(y, lessThanOrEqualTo(1 + easingHandleReach),
              reason: preset.id);
        }
      }
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

    test('a shape on the chord converts to the chord slope', () {
      // Handles at (1/3, 1/3) and (2/3, 2/3) lie on the diagonal, which is the
      // straight line — so both sides must come out travelling at exactly the
      // chord slope, the same thing anim.rs calls a linear side.
      final sides = EasingCurve(1 / 3, 1 / 3, 2 / 3, 2 / 3).sidesFor(7.5);
      expect(bez(sides.out)!.speed, closeTo(7.5, 1e-12));
      expect(bez(sides.inTo)!.speed, closeTo(7.5, 1e-12));
      expect(bez(sides.out)!.influence, closeTo(1 / 3, 1e-12));
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

    test('the in-side reach is measured back from the far end', () {
      // x2 is where the second control point sits along the span; the
      // influence stored on that key is how far it reaches *back*, so the two
      // are complements, not the same number.
      final sides = EasingCurve(0.2, 0, 0.9, 1).sidesFor(1);
      expect(bez(sides.out)!.influence, closeTo(0.2, 1e-12));
      expect(bez(sides.inTo)!.influence, closeTo(0.1, 1e-12));
    });

    test('a flat span stays flat whatever the shape', () {
      for (final preset in easingPresets) {
        final sides = preset.curve.sidesFor(0);
        expect(bez(sides.out)!.speed, 0, reason: preset.id);
        expect(bez(sides.inTo)!.speed, 0, reason: preset.id);
      }
    });

    test('every preset stores a legal influence', () {
      // anim.rs clamps influence into [1e-3, 1] on evaluation; a preset that
      // needed clamping would not be the shape it draws.
      for (final preset in easingPresets) {
        final sides = preset.curve.sidesFor(1);
        for (final side in [bez(sides.out)!, bez(sides.inTo)!]) {
          expect(side.influence, greaterThanOrEqualTo(minTangentReach),
              reason: preset.id);
          expect(side.influence, lessThanOrEqualTo(1), reason: preset.id);
        }
      }
    });

    test('the narrowest legal in-side reach is still a legal influence', () {
      // x2 clamped hard against its limit gives the smallest reach the type can
      // produce. Pinned as a test rather than trusted: it is the divisor inside
      // `sidesFor`, and it is what gets stored as influence, so if the two
      // bounds ever drift apart this is where it shows.
      final sides = EasingCurve(0.5, 0.5, 1, 1).sidesFor(1);
      expect(bez(sides.inTo)!.influence, greaterThanOrEqualTo(minTangentReach));
      expect(bez(sides.inTo)!.influence, closeTo(minTangentReach, 1e-9));
    });

    test('no preset draws a degenerate, directionless handle', () {
      // A handle of zero length has no direction to convert: the shape it
      // implies lives in the *next* control point, which (speed, influence)
      // cannot express. Shapes borrowed from CSS hit this — cubic-bezier(0, 0,
      // …) collapses onto the origin and converts to a flat start, the
      // opposite of what it draws.
      for (final preset in easingPresets) {
        expect(preset.curve.x1, greaterThan(minTangentReach),
            reason: preset.id);
        expect(preset.curve.x2, lessThan(1 - minTangentReach),
            reason: preset.id);
      }
    });
  });

  group('the drawn shape', () {
    test('runs corner to corner', () {
      for (final preset in easingPresets) {
        expect(preset.curve.xAt(0), closeTo(0, 1e-12), reason: preset.id);
        expect(preset.curve.yAt(0), closeTo(0, 1e-12), reason: preset.id);
        expect(preset.curve.xAt(1), closeTo(1, 1e-12), reason: preset.id);
        expect(preset.curve.yAt(1), closeTo(1, 1e-12), reason: preset.id);
      }
    });

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

    test('the overshoot preset leaves the box and comes back', () {
      final overshoot =
          easingPresets.firstWhere((p) => p.id == 'overshoot').curve;
      var peak = 0.0;
      for (var i = 0; i <= 64; i++) {
        peak = peak > overshoot.yAt(i / 64) ? peak : overshoot.yAt(i / 64);
      }
      expect(peak, greaterThan(1));
      expect(overshoot.yAt(1), closeTo(1, 1e-12));
    });

    test('the anticipate preset dips below the start and recovers', () {
      final anticipate =
          easingPresets.firstWhere((p) => p.id == 'anticipate').curve;
      var trough = 0.0;
      for (var i = 0; i <= 64; i++) {
        trough =
            trough < anticipate.yAt(i / 64) ? trough : anticipate.yAt(i / 64);
      }
      expect(trough, lessThan(0));
      expect(anticipate.yAt(1), closeTo(1, 1e-12));
    });
  });

  group('withHandle', () {
    test('moves one control point and leaves the other', () {
      final curve = EasingCurve(0.2, 0.3, 0.8, 0.7);
      final moved = curve.withHandle(first: true, x: 0.5, y: 0.9);
      expect(moved.x1, closeTo(0.5, 1e-12));
      expect(moved.y1, closeTo(0.9, 1e-12));
      expect(moved.x2, curve.x2);
      expect(moved.y2, curve.y2);
    });

    test('a handle dragged out of the box clamps both ways', () {
      final moved =
          EasingCurve(0.2, 0.3, 0.8, 0.7).withHandle(first: false, x: 2, y: 9);
      expect(moved.x2, 1 - minTangentReach);
      expect(moved.y2, 1 + easingHandleReach);
    });
  });
}
