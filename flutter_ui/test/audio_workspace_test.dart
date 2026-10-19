// The Audio workspace preset and the meter feed's own arithmetic — the plain
// half of the Audio panels package (no engine needed).

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/audio_meters_feed.dart';
import 'package:lumit_flutter/panels/audio_panel_frb.dart';
import 'package:lumit_flutter/panels/graph_maths.dart' show easyEase;
import 'package:lumit_flutter/src/rust/api/audio.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

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

  // The panel rewrites a fade by taking its two keys out first, so a wrong
  // read here would delete keys the user set.
  test('a fade is read off the Volume keys at its own end, and nothing else',
      () {
    BridgeKeyframe key(double t, double v, BridgeSideInterp a,
            [BridgeSideInterp? b]) =>
        BridgeKeyframe(
          time: BridgeRational(num: (t * 1000).round(), den: 1000),
          value: v,
          interpIn: a,
          interpOut: b ?? a,
        );
    const lin = BridgeSideInterp.linear();
    final volume = BridgeScalar.keyframed([
      key(0, -100, lin),
      key(1.5, -6, easyEase),
      key(4, -3, lin),
      key(8, -3, easyEase),
      key(10, -100, easyEase),
    ]);
    final fadeIn = audioFadeOf(volume,
        edgeSeconds: 0, rising: true, slack: 0.01);
    expect(fadeIn?.first, 0);
    expect(fadeIn?.seconds, closeTo(1.5, 1e-9));
    expect(fadeIn?.shape, BridgeFadeShape.exponential);
    expect(fadeIn?.level, -6);
    final fadeOut = audioFadeOf(volume,
        edgeSeconds: 10, rising: false, slack: 0.01);
    expect(fadeOut?.first, 3);
    expect(fadeOut?.seconds, closeTo(2, 1e-9));
    expect(fadeOut?.shape, BridgeFadeShape.ease);
    // A key at the edge that is not silence is a level, not a fade.
    expect(
        audioFadeOf(BridgeScalar.keyframed([key(0, -3, lin), key(2, 0, lin)]),
            edgeSeconds: 0, rising: true, slack: 0.01),
        isNull);
    expect(
        audioFadeOf(const BridgeScalar.static_(0),
            edgeSeconds: 0, rising: true, slack: 0.01),
        isNull);
  });
}
