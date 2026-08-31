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
    );

void main() {
  test('measuring is on to begin with, matching the engine', () {
    final asked = <bool>[];
    final timings = RenderTimings(askEngine: asked.add);
    expect(timings.measuring, isTrue,
        reason: 'numbers are what the column is for, and a switch nobody finds '
            'is a feature that does not work (K-276)');
    expect(asked, isEmpty,
        reason: 'the engine starts on too, so nothing is said at startup');

    timings.report(_profile());
    expect(timings.layerMs('layer-a'), isNotNull);
  });

  test('switched off, a profile that arrives anyway is ignored', () {
    final timings = RenderTimings(measuring: false, askEngine: (_) {});
    timings.report(_profile());
    expect(timings.layerMs('layer-a'), isNull);
  });

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

  test('an engine that refuses the switch leaves it off, and says so', () {
    // The state that reads as "this feature does not work": a lit switch over a
    // column that will never fill, because the engine never heard the ask.
    final errors = <Object>[];
    final timings = RenderTimings(
      measuring: false,
      askEngine: (on) => throw StateError('no such call'),
      onEngineError: errors.add,
    );

    timings.setMeasuring(true);
    expect(timings.measuring, isFalse,
        reason: 'the flag follows the engine, not the click');
    expect(errors, hasLength(1));
  });

  test('the frame total is null until a measured frame arrives', () {
    final timings = RenderTimings(askEngine: (_) {});
    expect(timings.totalMs, isNull,
        reason: 'the header shows … rather than a number it does not have');
    timings.report(_profile());
    expect(timings.totalMs, closeTo(31.5, 1e-9));
  });

  test('the stages arrive with the frame and leave with the switch', () {
    final timings = RenderTimings(askEngine: (_) {});
    expect(timings.stages, isEmpty,
        reason: 'no measured frame yet, nothing to split');
    timings.report(_profile());
    expect(timings.stages, hasLength(5));
    expect(timings.stages[3].kind, RenderStageKind.composite);
    expect(timings.stages[3].ms, closeTo(26.5, 1e-9));
    timings.setMeasuring(false);
    expect(timings.stages, isEmpty, reason: 'off drops the split too');
  });

  test('the header names a culprit only when the rows cannot explain it', () {
    List<RenderStageMs> stages(
            double plan, double decode, double build, double comp, double p) =>
        [
          RenderStageMs(RenderStageKind.plan, plan),
          RenderStageMs(RenderStageKind.decode, decode),
          RenderStageMs(RenderStageKind.build, build),
          RenderStageMs(RenderStageKind.composite, comp),
          RenderStageMs(RenderStageKind.present, p),
        ];
    // The ~97 ms class: the build owns the frame and no layer ever will.
    expect(dominantUnownedStage(stages(1, 2, 90, 3, 1)), RenderStageKind.build);
    // Compositing dominating is the rows' own story — the column itemises it.
    expect(dominantUnownedStage(stages(1, 2, 3, 90, 1)), isNull);
    // Nothing dominating names nobody: a guess would be worse than silence.
    expect(dominantUnownedStage(stages(10, 12, 11, 9, 8)), isNull);
    expect(dominantUnownedStage(const []), isNull);
  });

  test('the readout stays the same width and never lies about precision', () {
    expect(formatRenderMs(0), '0.00 ms');
    expect(formatRenderMs(8.24), '8.24 ms');
    expect(formatRenderMs(99.94), '99.94 ms');
    expect(formatRenderMs(100), '100 ms');
    expect(formatRenderMs(842.6), '843 ms');
    expect(formatRenderMs(1000), '1.00 s');
    expect(formatRenderMs(2543), '2.54 s');
    expect(formatRenderMs(double.nan), '—');
    expect(formatRenderMs(-1), '—');
  });
}
