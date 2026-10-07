// The Audio workspace preset and the meter feed's own arithmetic — the plain
// half of the Audio panels package (no engine needed).

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/audio_meters_feed.dart';
import 'package:lumit_flutter/src/rust/api/audio.dart';

void main() {
  group('Meter feed', () {
    BridgeAudioMeter meter(String layer, double peak, {bool clipped = false}) =>
        BridgeAudioMeter(
          layer: layer,
          peakLeft: peak,
          peakRight: peak,
          rmsLeft: peak * 0.7,
          rmsRight: peak * 0.7,
          clipped: clipped,
        );

    test('the hold rides the loudest peak and lets go after the hold time',
        () {
      final feed = AudioMeterFeed();
      addTearDown(feed.dispose);
      var reading = [meter('a', 0.5)];
      feed.read = () => reading;
      feed.tick();
      expect(feed.frame.value.of('a').holdLeft, 0.5);

      // Quieter now: the hold stays on the loudest peak seen.
      reading = [meter('a', 0.1)];
      feed.tick();
      expect(feed.frame.value.of('a').holdLeft, 0.5,
          reason: 'the line rests above the bar rather than falling with it');

      // Louder: the hold rises at once.
      reading = [meter('a', 0.8)];
      feed.tick();
      expect(feed.frame.value.of('a').holdLeft, 0.8);
    });
  });
}
