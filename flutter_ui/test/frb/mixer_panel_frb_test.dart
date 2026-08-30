// The Mixer panel against the real engine: the Master strip that is always
// there, the fader that writes the master gain (K-691), and the K-681 gate —
// a meter tick repaints the meter band and nothing else.

import 'package:flutter/gestures.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/audio_meters_feed.dart';
import 'package:lumit_flutter/panels/mixer_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/audio.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  /// A feed the test pulses by hand: never started, so no timer runs, and
  /// `read` answers whatever the test sets.
  AudioMeterFeed handFeed() {
    final feed = AudioMeterFeed();
    feed.read = () => const [];
    addTearDown(feed.dispose);
    return feed;
  }

  Future<({dynamic ui, dynamic comp, AudioMeterFeed feed})> mount(
      WidgetTester tester) async {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Mix');
    comp.addSolidLayer();
    p.uiState.setSelectedComp(comp);
    p.uiState.model.refresh();
    final feed = handFeed();
    await tester.pumpWidget(hostPanel(
      state: p.state,
      uiState: p.uiState,
      child: RepaintBoundary(
        key: const ValueKey('mixer-shell'),
        child: MixerPanelFrb(feed: feed),
      ),
    ));
    await settleFrb(tester, minRounds: 4);
    return (ui: p.uiState, comp: comp, feed: feed);
  }

  group('Mixer (frb)', () {
    /// The Master strip stands whatever the comp holds (K-691): fader, meter
    /// band, the limiter lamp and the muted LUFS placeholder — and a comp of
    /// silent layers says so where the strips would be.
    testWidgets('the Master strip is always on the desk', (tester) async {
      await mount(tester);
      expect(find.byKey(const ValueKey('fader-master')), findsOneWidget);
      expect(find.byKey(const ValueKey('meter-band-')), findsOneWidget);
      expect(find.byKey(const ValueKey('limiter-lamp')), findsOneWidget);
      expect(find.text('Master'), findsOneWidget);
      expect(find.text('LUFS —'), findsOneWidget,
          reason: 'loudness is post-v1, so the readout is the muted placeholder');
      expect(find.text('No layer in this composition makes a sound'),
          findsOneWidget,
          reason: 'a solid cannot be heard, so there is no strip for it');
    });

    /// Dragging the master fader writes `Composition.master_volume_db` — one
    /// commit on release, so one undo step.
    testWidgets('the master fader writes the master gain', (tester) async {
      final p = await mount(tester);
      expect(p.comp.masterVolumeDb(), 0.0);

      final fader = find.byKey(const ValueKey('fader-master'));
      final centre = tester.getCenter(fader);
      // A mouse, as on the desk this app ships for: touch slop swallows a
      // short test drag. Several moves, because the first only wins the
      // recogniser the arena and a real drag is a stream of them anyway.
      final gesture =
          await tester.startGesture(centre, kind: PointerDeviceKind.mouse);
      for (var i = 0; i < 4; i++) {
        await gesture.moveBy(const Offset(0, 10));
        await tester.pump();
      }
      await gesture.up();
      await tester.pump();

      final db = p.comp.masterVolumeDb();
      expect(db, lessThan(0.0),
          reason: 'pulling the fader down turns the sum down');
      expect(db, greaterThanOrEqualTo(-60.0));
      // The fader also answers a double-click (reset to unity), so its
      // recogniser holds the double-tap window open after the release; let it
      // lapse rather than ending the test with its timer pending.
      await tester.pump(kDoubleTapTimeout);
    });

    /// The K-681 gate this panel ships with: a meter tick repaints the meter
    /// band inside its own boundary and repaints **nothing else** — the strips,
    /// wells and faders never hear the timer.
    testWidgets('a meter tick repaints only the meter band', (tester) async {
      final p = await mount(tester);

      int paints(String key) {
        final boundary = tester.renderObject<RenderRepaintBoundary>(
            find.byKey(ValueKey<String>(key)).first);
        return boundary.debugSymmetricPaintCount +
            boundary.debugAsymmetricPaintCount;
      }

      final bandBefore = paints('meter-band-');
      final shellBefore = paints('mixer-shell');

      for (var i = 1; i <= 10; i++) {
        p.feed.read = () => [
              BridgeAudioMeter(
                layer: '',
                peakLeft: i / 10,
                peakRight: i / 10,
                rmsLeft: i / 20,
                rmsRight: i / 20,
                clipped: false,
              ),
            ];
        p.feed.tick();
        await tester.pump(const Duration(milliseconds: 16));
      }

      expect(paints('meter-band-'), greaterThan(bandBefore),
          reason: 'the bars did not redraw, so nothing was measured');
      expect(paints('mixer-shell'), shellBefore,
          reason: 'a meter tick redrew the panel around the bars');
    });

    /// The clip light is the lamp's own listenable: it lights on a clipped
    /// poll without the strips rebuilding, and clicking it asks the engine to
    /// put the lights out (a calm no-op when nothing is loaded).
    testWidgets('the limiter lamp follows the clip flag', (tester) async {
      final p = await mount(tester);
      expect(p.feed.clipped.value, isFalse);
      p.feed.read = () => const [
            BridgeAudioMeter(
              layer: '',
              peakLeft: 1,
              peakRight: 1,
              rmsLeft: 1,
              rmsRight: 1,
              clipped: true,
            ),
          ];
      p.feed.tick();
      await tester.pump();
      expect(p.feed.clipped.value, isTrue);
      await tester.tap(find.byKey(const ValueKey('limiter-lamp')));
      await tester.pump();
      // The engine has no mix loaded in a test, so the reset is a no-op —
      // the point is that the tap route exists and does not throw.
    });
  });
}
