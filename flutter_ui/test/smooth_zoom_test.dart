
// The shared zoom motion (docs/07-UI-SPEC.md §4.6): the acceleration rule on
// its own, and the flight it drives.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/widgets/smooth_zoom.dart';

void main() {
  group('The flight', () {
    testWidgets('arrives where it was sent', (tester) async {
      final zoom = SmoothZoom(vsync: tester, initial: 1);
      addTearDown(zoom.dispose);

      zoom.goTo(4);
      expect(zoom.target, 4);
      expect(zoom.value, 1, reason: 'it has not moved yet');
      await tester.pumpAndSettle();
      expect(zoom.value, closeTo(4, 1e-6));
      expect(zoom.moving, isFalse, reason: 'and it stops when it arrives');
    });

    testWidgets('is held inside its bounds', (tester) async {
      final zoom = SmoothZoom(vsync: tester, initial: 1, min: 0.5, max: 8);
      addTearDown(zoom.dispose);

      zoom.goTo(1000);
      expect(zoom.target, 8);
      zoom.goTo(0.001);
      expect(zoom.target, 0.5);
      await tester.pumpAndSettle();
    });

    /// **A notch inside a flight adds to the journey**, rather than restarting
    /// it from where the flight had reached — which is what makes a rolled
    /// wheel one continuous motion instead of a series of short hops that never
    /// get anywhere.
    testWidgets('a second notch extends the target rather than resetting it',
        (tester) async {
      var now = Duration.zero;
      final zoom = SmoothZoom(vsync: tester, initial: 1, clock: () => now);
      addTearDown(zoom.dispose);

      // Two notches a long way apart: no boost, so each is worth exactly 2×.
      zoom.nudge(2);
      now += const Duration(seconds: 1);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 40));
      zoom.nudge(2);
      expect(zoom.target, closeTo(4, 1e-6),
          reason: 'the second notch doubled the target, not the value it had '
              'reached part-way through the first flight');
      await tester.pumpAndSettle();
    });

    testWidgets('a rolled wheel goes further than the same notches clicked',
        (tester) async {
      var now = Duration.zero;
      SmoothZoom build() =>
          SmoothZoom(vsync: tester, initial: 1, clock: () => now);

      final clicked = build();
      addTearDown(clicked.dispose);
      for (var i = 0; i < 3; i++) {
        now += const Duration(milliseconds: 400);
        clicked.nudge(1.12);
      }
      final clickedTarget = clicked.target;

      now = Duration.zero;
      final rolled = build();
      addTearDown(rolled.dispose);
      for (var i = 0; i < 3; i++) {
        now += const Duration(milliseconds: 15);
        rolled.nudge(1.12);
      }
      expect(rolled.target, greaterThan(clickedTarget),
          reason: 'the same three notches, rolled, cover more ground');
      await tester.pumpAndSettle();
    });
  });

  /// The Timeline's zoom slider (owner, 2026-08-06): its left end is the whole
  /// composition, its right end is twenty frames across the lanes.
  group('Where the zoom slider sits', () {
    // A 1200-frame comp: twenty frames across the lanes is 60x.
    const maxZoom = 60.0;

    test('it round-trips, so dragging the handle does not drift', () {
      for (final t in [0.0, 0.13, 0.5, 0.77, 1.0]) {
        expect(
          zoomSliderPosition(zoomForSliderPosition(t, maxZoom), maxZoom),
          closeTo(t, 1e-9),
        );
      }
    });
  });
}