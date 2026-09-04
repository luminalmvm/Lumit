// The render-time indicators against the real engine (docs/13 §7.1).
//
// **The bug this exists for.** The column shipped drawing nothing at all, on
// every platform, and the reason was invisible from either side alone: numbers
// exist only for a frame the engine *composites*, and a frame the cache already
// holds is served without compositing. So a composition warm enough to be worth
// profiling — which is every composition, a moment after it is opened, because
// the idle fill makes frames while you think — answered every render from the
// cache and reported nothing. Measuring now steps over the tiers, which is the
// cost of asking and the whole point of asking. (Measuring is on by default
// now, and the clock in the bottom strip turns it off.)
//
// Two things are pinned here, and each fails without its half of the fix: that
// a measured render reports at all, and that the ids it reports are the ones
// the panels look the numbers up by.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Render-time indicators (frb)', () {
    /// A small comp with one solid layer carrying one effect, selected — small
    /// so a software rasteriser makes each frame quickly.
    ({dynamic p, CompositionReference comp, String layerId, String effectId})
        withEffect() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      final was = comp.getSettings();
      comp.setSettings(
        settings: BridgeCompSettings(
          name: was.name,
          width: 64,
          height: 36,
          fpsNum: was.fpsNum,
          fpsDen: was.fpsDen,
          duration: was.duration,
          background: was.background,
          shutterAngle: was.shutterAngle,
          motionBlurSamples: was.motionBlurSamples,
        ),
      );
      comp.addSolidLayer();
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'blur');
      p.uiState.setSelectedComp(comp);
      return (
        p: p,
        comp: comp,
        layerId: layer.internallayerId.toString(),
        effectId: layer.getEffects().single.id().toString(),
      );
    }

    testWidgets('a measured frame reports what its layer and its effect cost',
        (tester) async {
      final f = withEffect();
      final timings = f.p.uiState.renderTimings;

      final profiles = <BridgeFrameProfile>[];
      final sub = f.p.state.onWorkerResponse.listen((msg) {
        if (msg is WorkerResponse_FrameProfile) profiles.add(msg.field0);
      });
      addTearDown(sub.cancel);

      expect(timings.measuring, isTrue, reason: 'on by default');

      // Ask for the frame twice. The second would be a cache hit — the state
      // the column used to die in — and must be measured all the same.
      for (var i = 0; i < 2; i++) {
        f.comp.renderFrame(
          frame: BigInt.zero,
          scale: 1.0,
          mode: BridgePlaybackMode.everyFrame,
        );
        // Wait for *this* ask's profile - `isNotEmpty` was already true on
        // the second pass, so the loop never waited for the cache hit's
        // profile and the count assertion below raced it.
        await tester.runAsync(() async {
          for (var round = 0; round < 100; round++) {
            await Future<void>.delayed(const Duration(milliseconds: 100));
            if (profiles.length > i) return;
          }
        });
        await settleFrb(tester, minRounds: 4, maxRounds: 30);
      }

      expect(profiles, isNotEmpty,
          reason: 'a held frame is re-composited while measuring, or there is '
              'nothing to measure and the column stays empty for ever');
      expect(profiles.length, greaterThanOrEqualTo(2),
          reason: 'the second ask was a cache hit and was measured too');
      final profile = profiles.last;
      expect(profile.totalMs, greaterThanOrEqualTo(0));
      expect(profile.layers, isNotEmpty);

      // The ids must be the strings the panels look up by — the layer's own id
      // as the Timeline row knows it, and the effect instance's as the Effect
      // controls heading knows it.
      expect(timings.layerMs(f.layerId), isNotNull,
          reason: 'the Timeline row finds its layer');
      expect(timings.effectMs(f.effectId), isNotNull,
          reason: 'the effect heading finds its effect');
      expect(timings.layerMs(f.layerId)!, greaterThanOrEqualTo(0));
      expect(timings.frame, 0);
    });

    testWidgets('switching measuring off drops the numbers and goes quiet',
        (tester) async {
      final f = withEffect();
      final timings = f.p.uiState.renderTimings;

      f.comp.renderFrame(
        frame: BigInt.zero,
        scale: 1.0,
        mode: BridgePlaybackMode.everyFrame,
      );
      await tester.runAsync(() async {
        for (var i = 0; i < 150; i++) {
          await Future<void>.delayed(const Duration(milliseconds: 100));
          if (timings.layerMs(f.layerId) != null) return;
        }
      });
      await settleFrb(tester, minRounds: 4, maxRounds: 20);
      expect(timings.layerMs(f.layerId), isNotNull);

      timings.setMeasuring(false);
      expect(timings.measuring, isFalse);
      expect(timings.layerMs(f.layerId), isNull,
          reason: 'a stale cost reads as a live one, so it is dropped');

      // And a render afterwards leaves it that way: the engine is not
      // measuring, so nothing arrives to put numbers back.
      f.comp.renderFrame(
        frame: BigInt.one,
        scale: 1.0,
        mode: BridgePlaybackMode.everyFrame,
      );
      await settleFrb(tester, minRounds: 10, maxRounds: 40);
      expect(timings.layerMs(f.layerId), isNull);
    });
  });
}
