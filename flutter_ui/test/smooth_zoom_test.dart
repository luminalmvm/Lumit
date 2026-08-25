import 'dart:math' as math;

// The shared zoom motion (docs/07-UI-SPEC.md §4.6): the acceleration rule on
// its own, and the flight it drives.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/widgets/smooth_zoom.dart';

void main() {
  group('How much a notch is worth', () {
    test('a notch on its own is worth exactly itself', () {
      expect(zoomBoost(smoothZoomFastGap), 1);
      expect(zoomBoost(const Duration(seconds: 1)), 1);
    });

    test('notches arriving faster are worth more', () {
      final slow = zoomBoost(const Duration(milliseconds: 100));
      final quick = zoomBoost(const Duration(milliseconds: 40));
      final flick = zoomBoost(const Duration(milliseconds: 5));
      expect(slow, greaterThan(1));
      expect(quick, greaterThan(slow));
      expect(flick, greaterThan(quick));
    });

    test('however hard the wheel is rolled, there is a ceiling', () {
      // Without one a flick crosses the whole range and there is no way back
      // to where you were.
      expect(zoomBoost(Duration.zero), smoothZoomMaxBoost);
      expect(zoomBoost(const Duration(microseconds: -5)), smoothZoomMaxBoost);
      for (final ms in [0, 1, 5, 20, 60, 119]) {
        expect(zoomBoost(Duration(milliseconds: ms)),
            lessThanOrEqualTo(smoothZoomMaxBoost));
      }
    });

    test('the curve is continuous at the fast/slow boundary', () {
      expect(zoomBoost(const Duration(milliseconds: 119)), closeTo(1, 0.03));
    });
  });

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

    testWidgets('moves geometrically, so equal time buys equal ratio',
        (tester) async {
      final zoom = SmoothZoom(vsync: tester, initial: 1);
      addTearDown(zoom.dispose);

      zoom.goTo(16);
      await tester.pump();
      await tester.pump(smoothZoomFlight ~/ 2);
      final half = zoom.value;
      addTearDown(() async => tester.pumpAndSettle());
      // Half way through a 1 → 16 flight is 4, not 8.5: the logarithm is what
      // is lerped, because magnification is a ratio.
      expect(half, closeTo(4, 0.2));
      await tester.pumpAndSettle();
    });

    testWidgets('a zero-length flight arrives at once, for reduced motion',
        (tester) async {
      final zoom = SmoothZoom(vsync: tester, initial: 1);
      addTearDown(zoom.dispose);

      zoom.goTo(4, duration: Duration.zero);
      expect(zoom.value, 4);
      expect(zoom.moving, isFalse);
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

    testWidgets('and it settles when the hand stops', (tester) async {
      var now = Duration.zero;
      final zoom = SmoothZoom(vsync: tester, initial: 1, clock: () => now);
      addTearDown(zoom.dispose);

      for (var i = 0; i < 5; i++) {
        now += const Duration(milliseconds: 15);
        zoom.nudge(1.12);
      }
      final settled = zoom.target;
      await tester.pumpAndSettle();
      expect(zoom.moving, isFalse);
      expect(zoom.value, closeTo(settled, 1e-6),
          reason: 'the flight finishes rather than stopping where the last '
              'notch left it');
    });
  });


  /// The Timeline's zoom slider (owner, 2026-08-06): its left end is the whole
  /// composition, its right end is twenty frames across the lanes.
  group('Where the zoom slider sits', () {
    // A 1200-frame comp: twenty frames across the lanes is 60x.
    const maxZoom = 60.0;

    test('the far left is the whole composition', () {
      expect(zoomSliderPosition(1, maxZoom), 0);
      expect(zoomForSliderPosition(0, maxZoom), 1);
    });

    test('the far right is full zoom', () {
      expect(zoomSliderPosition(maxZoom, maxZoom), 1);
      expect(zoomForSliderPosition(1, maxZoom), closeTo(maxZoom, 1e-9));
    });

    test('it round-trips, so dragging the handle does not drift', () {
      for (final t in [0.0, 0.13, 0.5, 0.77, 1.0]) {
        expect(
          zoomSliderPosition(zoomForSliderPosition(t, maxZoom), maxZoom),
          closeTo(t, 1e-9),
        );
      }
    });

    /// **Equal travel buys equal ratio.** A linear slider would spend nine
    /// tenths of its length inside the last handful of frames of a long comp,
    /// crushing every useful zoom into the first centimetre.
    test('the middle of the slider is the geometric middle of the range', () {
      expect(zoomForSliderPosition(0.5, maxZoom),
          closeTo(math.sqrt(maxZoom), 1e-9));
      // And a quarter of the way along is the fourth root, not a quarter of 60.
      expect(zoomForSliderPosition(0.25, maxZoom),
          closeTo(math.pow(maxZoom, 0.25), 1e-9));
    });

    test('a value outside the range is held inside it', () {
      expect(zoomSliderPosition(0.1, maxZoom), 0);
      expect(zoomSliderPosition(1000, maxZoom), 1);
      expect(zoomForSliderPosition(-1, maxZoom), 1);
      expect(zoomForSliderPosition(2, maxZoom), closeTo(maxZoom, 1e-9));
    });

    test('a comp already shorter than full zoom has nowhere to travel', () {
      // Fewer than twenty frames: the ceiling is 1, and the handle sits left
      // rather than dividing by a logarithm of one.
      expect(zoomSliderPosition(1, 1), 0);
      expect(zoomForSliderPosition(0.5, 1), 1);
      expect(zoomForSliderPosition(1, 0.5), 1);
    });

    /// The promise the ceiling makes: at the right-hand end you are looking at
    /// twenty frames, whatever the composition's length. The visible span is
    /// `frames / zoom`, so this is the arithmetic that has to hold.
    test('full zoom shows twenty frames, whatever the comp is', () {
      for (final frames in [200, 1200, 18000]) {
        final ceiling = math.max(1.0, frames / 20);
        final visible = frames / zoomForSliderPosition(1, ceiling);
        expect(visible, closeTo(20, 1e-6), reason: '$frames frames');
      }
    });
  });
}