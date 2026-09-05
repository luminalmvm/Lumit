// An angle reads as turns AND degrees, and the pair is only ever a *view* of
// one number (docs/07 §6.1).
//
// The split matters because 30° and 390° are the same picture but not the same
// animation: a key at 30 followed by a key at 390 travels a whole turn, and one
// at 30 followed by one at 30 does not move. What is stored stays the single
// angle, so the property animates and serialises exactly as it always did —
// which is why these are pure-function tests rather than widget ones. If the
// split and the recombination ever disagree, a rotation drifts by whole turns
// every time someone touches its row.

import 'dart:math' as math;

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/angle_dial.dart';
import 'package:lumit_flutter/widgets/controls.dart';

void main() {
  group('turns and degrees', () {
    /// Every case round-trips: whatever the split says, putting it back has to
    /// give the number that went in.
    void roundTrips(double value) {
      final turns = TurnsAndDegreesField.turnsOf(value);
      final degrees = TurnsAndDegreesField.degreesOf(value);
      expect(turns * 360 + degrees, closeTo(value, 1e-9),
          reason: '$value split to ${turns}x $degrees and did not come back');
    }

    test('splits toward zero, the way a rotation is spoken', () {
      // Under a turn: no turns at all.
      expect(TurnsAndDegreesField.turnsOf(30), 0);
      expect(TurnsAndDegreesField.degreesOf(30), closeTo(30, 1e-9));

      // A turn and a bit.
      expect(TurnsAndDegreesField.turnsOf(390), 1);
      expect(TurnsAndDegreesField.degreesOf(390), closeTo(30, 1e-9));

      // Exactly two turns is two turns and nothing.
      expect(TurnsAndDegreesField.turnsOf(720), 2);
      expect(TurnsAndDegreesField.degreesOf(720), closeTo(0, 1e-9));

      // **Negatives truncate toward zero, not downward.** −370 is "minus one
      // turn and minus ten", which is how it is read aloud; flooring would call
      // it −2 turns and +350, which is the same angle and the wrong sentence.
      expect(TurnsAndDegreesField.turnsOf(-370), -1);
      expect(TurnsAndDegreesField.degreesOf(-370), closeTo(-10, 1e-9));
      expect(TurnsAndDegreesField.turnsOf(-30), 0);
      expect(TurnsAndDegreesField.degreesOf(-30), closeTo(-30, 1e-9));
    });

    test('round-trips whatever it is given', () {
      for (final v in <double>[
        0,
        0.5,
        -0.5,
        30,
        -30,
        359.9,
        360,
        -360,
        390,
        -390,
        719.5,
        -1080,
        36000.25,
      ]) {
        roundTrips(v);
      }
    });

    test('a value the split cannot represent does not exist', () {
      // The degrees half is always strictly inside a turn, so the two fields
      // can never both be at their extremes and mean something ambiguous.
      for (var i = -2000; i <= 2000; i += 7) {
        final v = i * 0.37;
        expect(TurnsAndDegreesField.degreesOf(v).abs(), lessThan(360));
      }
    });
  });

  group('the dial winds through full turns', () {
    const size = 100.0;

    /// A dial at [degrees] in a box at the origin, reporting every value the
    /// drag hands back.
    Future<List<double>> mount(WidgetTester tester, double degrees) async {
      final seen = <double>[];
      await tester.pumpWidget(Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Align(
            alignment: Alignment.topLeft,
            child: AngleDial(
              size: size,
              degrees: degrees,
              onChanged: seen.add,
              onChangeEnd: seen.add,
            ),
          ),
        ),
      ));
      return seen;
    }

    /// The point on the dial's rim [clockwise] degrees from twelve o'clock.
    Offset rim(double clockwise) {
      final a = clockwise * math.pi / 180;
      return Offset(size / 2 + math.sin(a) * 40, size / 2 - math.cos(a) * 40);
    }

    /// One drag from twelve o'clock through [turns] whole turns in 45° steps,
    /// clockwise for a positive count and anticlockwise for a negative one.
    Future<void> wind(WidgetTester tester, double turns) async {
      final gesture = await tester.startGesture(rim(0));
      final steps = (turns.abs() * 8).round();
      for (var i = 1; i <= steps; i++) {
        await gesture.moveTo(rim(turns.sign * i * 45));
        await tester.pump();
      }
      await gesture.up();
      await tester.pump();
    }

    /// Dragging the hand round once used to hand back the angle it started
    /// from: every move was measured against the start and folded to within
    /// half a turn, so the turns box never moved off 0x.
    testWidgets('a clockwise drag past twelve adds a turn', (tester) async {
      final seen = await mount(tester, 30);
      await wind(tester, 1);
      expect(seen.last, closeTo(390, 1e-6));
      expect(TurnsAndDegreesField.turnsOf(seen.last), 1);
    });

    testWidgets('an anticlockwise drag takes turns away', (tester) async {
      final seen = await mount(tester, 30);
      await wind(tester, -2);
      expect(seen.last, closeTo(-690, 1e-6));
      expect(TurnsAndDegreesField.turnsOf(seen.last), -1);
    });

    testWidgets('the hand still follows the pointer inside a turn',
        (tester) async {
      final seen = await mount(tester, 0);
      final gesture = await tester.startGesture(rim(0));
      await gesture.moveTo(rim(90));
      await tester.pump();
      await gesture.moveTo(rim(45));
      await tester.pump();
      await gesture.up();
      await tester.pump();
      expect(seen.last, closeTo(45, 1e-6));
    });
  });

  group('the degrees box crossing 360', () {
    /// The pair at [degrees], reporting every value a drag hands back. Each
    /// value is fed straight back in as the new angle, the way the row does
    /// with a live tick, since that rebuild is what the bug lived in.
    Future<List<double>> mount(WidgetTester tester, double degrees) async {
      final seen = <double>[];
      var shown = degrees;
      await tester.pumpWidget(Directionality(
        textDirection: TextDirection.ltr,
        child: ThemeScope(
          theme: LumitTheme.dark(),
          animationLevel: AnimationLevel.none,
          showTooltips: false,
          child: Align(
            alignment: Alignment.topLeft,
            child: StatefulBuilder(
              builder: (context, setState) => TurnsAndDegreesField(
                keyName: 't',
                degrees: shown,
                onChanged: (v) => setState(() => seen.add(shown = v)),
                onCommit: (v) => setState(() => seen.add(shown = v)),
              ),
            ),
          ),
        ),
      ));
      return seen;
    }

    /// Scrubbing the degrees box from 350 up through 360 used to send the
    /// turns box racing: the box runs on from its own last tick, so it
    /// reported 361, 362, and the row added the turn it had just gained on
    /// top of each one. One turn is gained once, at 360, and no more.
    testWidgets('scrubbing past 360 gains exactly one turn', (tester) async {
      final seen = await mount(tester, 350);
      final box = find.byKey(const ValueKey<String>('angle-degrees-t'));
      final gesture = await tester.startGesture(tester.getCenter(box));
      // Past the drag slop first, then the scrub proper in small steps.
      await gesture.moveBy(const Offset(20, 0));
      await tester.pump();
      for (var i = 0; i < 10; i++) {
        await gesture.moveBy(const Offset(2, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pump();

      expect(seen, isNotEmpty);
      for (var i = 1; i < seen.length; i++) {
        expect(seen[i] - seen[i - 1], inInclusiveRange(0, 30),
            reason: 'tick $i jumped from ${seen[i - 1]} to ${seen[i]}');
      }
      expect(seen.last, inInclusiveRange(360, 400));
      expect(TurnsAndDegreesField.turnsOf(seen.last), 1);
    });

    testWidgets('scrubbing down through 0 loses exactly one turn',
        (tester) async {
      final seen = await mount(tester, 370);
      final box = find.byKey(const ValueKey<String>('angle-degrees-t'));
      final gesture = await tester.startGesture(tester.getCenter(box));
      await gesture.moveBy(const Offset(-20, 0));
      await tester.pump();
      for (var i = 0; i < 10; i++) {
        await gesture.moveBy(const Offset(-2, 0));
        await tester.pump();
      }
      await gesture.up();
      await tester.pump();

      expect(seen, isNotEmpty);
      for (var i = 1; i < seen.length; i++) {
        expect(seen[i - 1] - seen[i], inInclusiveRange(0, 30),
            reason: 'tick $i jumped from ${seen[i - 1]} to ${seen[i]}');
      }
      expect(seen.last, inInclusiveRange(320, 360));
      expect(TurnsAndDegreesField.turnsOf(seen.last), 0);
    });
  });
}
