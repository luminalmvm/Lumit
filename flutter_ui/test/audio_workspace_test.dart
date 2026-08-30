// The Audio workspace preset and the meter feed's own arithmetic — the plain
// half of the Audio panels package (no engine needed).

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/audio_meters_feed.dart';
import 'package:lumit_flutter/src/rust/api/audio.dart';
import 'package:lumit_flutter/state/dock.dart';

void main() {
  group('Audio workspace preset', () {
    /// The board's arrangement: Mixer fronting the left column over Project
    /// and Effect controls, the Audio panel fronting the right over
    /// Effects & presets, the Viewer between, the Timeline across the bottom.
    test('the preset holds the board\'s panels, fronted as drawn', () {
      final layout = presetLayout(WorkspacePreset.audio);
      final panels = panelsIn(layout);
      expect(
        panels,
        containsAll([
          Panel.mixer,
          Panel.audio,
          Panel.viewer,
          Panel.timeline,
          Panel.project,
          Panel.effectControls,
          Panel.effectsAndPresets,
        ]),
      );

      final upper = layout.children.first as DockSplit;
      final left = upper.children.first as DockTabs;
      expect(left.activePane.panel, Panel.mixer,
          reason: 'the board fronts the Mixer on the left column');
      final right = upper.children.last as DockTabs;
      expect(right.activePane.panel, Panel.audio,
          reason: 'the board fronts the Audio panel on the right column');
      expect(layout.children.last, isA<DockPane>(),
          reason: 'the Timeline runs the full width alone');
    });

    /// A saved arrangement from a build that predates these panels still
    /// opens (the dock's own resilience rule) — and one naming them reads
    /// back.
    test('the preset round-trips through JSON', () {
      final layout = presetLayout(WorkspacePreset.audio);
      final read = DockNode.fromJson(layout.toJson());
      expect(read, isA<DockSplit>());
      expect(panelsIn(read! as DockSplit), panelsIn(layout));
    });
  });

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

    test('the clip light is its own listenable and a silent poll is silent',
        () {
      final feed = AudioMeterFeed();
      addTearDown(feed.dispose);
      feed.read = () => [meter('', 0.9, clipped: true)];
      feed.tick();
      expect(feed.clipped.value, isTrue);
      expect(feed.frame.value.clipped, isTrue);

      feed.read = () => const [];
      feed.tick();
      expect(feed.clipped.value, isFalse);
      expect(feed.frame.value.of(''), StripLevels.silence,
          reason: 'a paused transport reads silence, not the last frame');
    });

    test('the dB scale puts full scale at the top and silence at the floor',
        () {
      expect(meterFraction(1.0), 1.0);
      expect(meterFraction(0.0), 0.0);
      // −20 dB is an amplitude of 0.1, two thirds of the way up a −60..0 bar.
      expect(meterFraction(0.1), closeTo(2 / 3, 1e-9));
      expect(amplitudeDb(0.5), closeTo(-6.02, 0.01));
    });
  });
}
