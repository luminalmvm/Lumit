// An angle reads as turns AND degrees, and the pair is only ever a *view* of
// one number (docs/07 §6.1, K-315).
//
// The split matters because 30° and 390° are the same picture but not the same
// animation: a key at 30 followed by a key at 390 travels a whole turn, and one
// at 30 followed by one at 30 does not move. What is stored stays the single
// angle, so the property animates and serialises exactly as it always did —
// which is why these are pure-function tests rather than widget ones. If the
// split and the recombination ever disagree, a rotation drifts by whole turns
// every time someone touches its row.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/widgets/angle_dial.dart';

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
}
