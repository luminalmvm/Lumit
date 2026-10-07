// The render-time indicators: the switch that asks the engine to measure, and
// what the numbers read as. The switch is the part with teeth — measuring costs
// real time on every frame it touches, so "off means off, and off drops the
// numbers" is a promise rather than a detail.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/src/rust/api/state.dart';
import 'package:lumit_flutter/state/render_timings.dart';

BridgeFrameProfile _profile() => BridgeFrameProfile(
      frame: BigInt.from(48),
      totalMs: 31.5,
      planMs: 0.5,
      decodeMs: 2.0,
      buildMs: 1.5,
      compositeMs: 26.5,
      presentMs: 1.0,
      layers: [
        BridgeLayerTiming(
          layer: 'layer-a',
          ms: 24.0,
          effects: [
            BridgeEffectTiming(effect: 'fx-blur', ms: 18.5),
            BridgeEffectTiming(effect: 'fx-glow', ms: 4.0),
          ],
        ),
        const BridgeLayerTiming(layer: 'layer-b', ms: 2.25, effects: []),
      ],
      view: 0,
    );

void main() {
  test('measuring gathers the numbers, and stopping drops them', () {
    final asked = <bool>[];
    final timings = RenderTimings(measuring: false, askEngine: asked.add);

    timings.setMeasuring(true);
    expect(asked, [true]);
    timings.report(_profile());

    expect(timings.frame, 48);
    expect(timings.totalMs, closeTo(31.5, 1e-9));
    expect(timings.layerMs('layer-a'), closeTo(24.0, 1e-9));
    expect(timings.layerMs('layer-b'), closeTo(2.25, 1e-9));
    expect(timings.effectMs('fx-blur'), closeTo(18.5, 1e-9));
    expect(timings.effectMs('fx-glow'), closeTo(4.0, 1e-9));
    // A layer the measured frame did not draw — hidden, out of its span, or
    // inside a Precomp — has no number rather than a wrong one.
    expect(timings.layerMs('layer-c'), isNull);

    timings.setMeasuring(false);
    expect(asked, [true, false]);
    expect(timings.layerMs('layer-a'), isNull,
        reason: 'a stale cost is worse than none');
    expect(timings.frame, isNull);

    // Asking twice for the same state does not pester the engine.
    timings.setMeasuring(false);
    expect(asked, [true, false]);
  });
}
