// The graph editor's pure maths, against hand-computed values and the
// engine's own constants (crates/lumit-core/src/anim.rs — the two
// implementations are pinned to docs/impl/keyframe-eval.md together).

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/graph_maths.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

BridgeRational rat(int n, int d) => BridgeRational(num: n, den: d);

BridgeKeyframe key(
  int n,
  int d,
  double v, {
  BridgeSideInterp interpIn = const BridgeSideInterp.linear(),
  BridgeSideInterp interpOut = const BridgeSideInterp.linear(),
}) =>
    BridgeKeyframe(
        time: rat(n, d), value: v, interpIn: interpIn, interpOut: interpOut);

void main() {
  envelopeTests();
  envelopeShapeTests();
  tangentModeTests();

  /// **M19 — a new keyframe takes after its neighbours.** It used to be born
  /// linear on both sides whatever it landed between, so planting one in the
  /// middle of a held run quietly turned that run into a ramp.
  group('keyframeAmong', () {
    const hold = BridgeSideInterp.hold();
    const ease =
        BridgeSideInterp.bezier(BridgeBezierSide(speed: 0, influence: 0.5));

    test('each half matches the side it faces', () {
      final keys = [
        key(0, 1, 0, interpOut: hold),
        key(2, 1, 10, interpIn: ease),
      ];
      final made = keyframeAmong(keys, rat(1, 1), 5);
      expect(made.interpIn, isA<BridgeSideInterp_Hold>(),
          reason: 'it arrives out of a hold');
      expect(made.interpOut, isA<BridgeSideInterp_Bezier>(),
          reason: 'and leaves into an ease');
    });
  });

  group('evaluateKeys', () {
    test('the easy-ease midpoint is the value midpoint', () {
      // Symmetric flat handles: the curve is odd about the centre, so the
      // midpoint in time is exactly the midpoint in value (anim.rs test).
      final keys = [
        key(0, 1, 0, interpOut: easyEase),
        key(1, 1, 100, interpIn: easyEase),
      ];
      expect(evaluateKeys(keys, 0.5), closeTo(50, 1e-9));
      // Eased: barely moving near the ends compared to linear.
      expect(evaluateKeys(keys, 0.1), lessThan(5));
      expect(evaluateKeys(keys, 0.9), greaterThan(95));
    });

    test('solveU round-trips x(u) = t', () {
      final cubic = CubicSpan.fromAe(0, 0, 1, 100,
          speedOut: 0, inflOut: 1.0, speedIn: 0, inflIn: 1.0);
      // Influence 1.0 both sides: dx/du = 0 at the endpoints — the spike case
      // the bracketed solve exists for.
      for (final t in [0.0, 0.001, 0.25, 0.5, 0.75, 0.999, 1.0]) {
        final u = cubic.solveU(t);
        expect(u, inInclusiveRange(0, 1));
        final x = 3 * (1 - u) * (1 - u) * u * cubic.x[1] +
            3 * (1 - u) * u * u * cubic.x[2] +
            u * u * u;
        expect(x, closeTo(t, 1e-9));
      }
    });
  });

  group('handle geometry', () {
    test('handleFromDrag inverts handleEndpoint', () {
      final e = handleEndpoint(
        keyTime: 2,
        keyValue: 10,
        neighbourTime: 4,
        isOut: true,
        speed: 15,
        influence: 0.4,
      );
      expect(e.time, closeTo(2.8, 1e-12));
      expect(e.value, closeTo(10 + 15 * 0.8, 1e-12));
      final back = handleFromDrag(
        keyTime: 2,
        keyValue: 10,
        neighbourTime: 4,
        isOut: true,
        dragTime: e.time,
        dragValue: e.value,
      );
      expect(back.speed, closeTo(15, 1e-9));
      expect(back.influence, closeTo(0.4, 1e-9));
    });
  });

  group('fit ranges', () {
    test('frames the keys, the handles and the overshoot, padded', () {
      final keys = [
        key(0, 1, 0,
            interpOut: const BridgeSideInterp.bezier(
                BridgeBezierSide(speed: 400, influence: 0.5))),
        key(1, 1, 10, interpIn: easyEase),
      ];
      final (lo, hi) = fitValueRange([keys], []);
      expect(lo, lessThanOrEqualTo(0));
      // The steep out-handle reaches 400 · 0.5 = 200 above the first key.
      expect(hi, greaterThanOrEqualTo(200));
    });
  });

  group('keyframe clipboard text', () {
    /// The whole reason the format is ours: a shaped key must come back
    /// shaped, not flattened to a straight line.
    test('easing survives the round trip, per column', () {
      const eased = BridgeSideInterp.bezier(
          BridgeBezierSide(speed: 12.5, influence: 0.25));
      final text = lumitClipboardText(
        version: '0.1.0',
        fps: 24,
        width: 1920,
        height: 1080,
        groups: [
          const LumitClipGroup(
            property: ['Transform', 'Position'],
            columns: ['X pixels', 'Y pixels'],
            rows: [
              LumitClipRow(
                frame: 12,
                values: [10, 20],
                eases: [
                  (eased, BridgeSideInterp.hold()),
                  (BridgeSideInterp.linear(), eased),
                ],
              ),
            ],
          ),
        ],
      );
      expect(text, contains('X pixels$easeInSuffix'));
      expect(text, contains('bezier(12.5,0.25)'));

      final row = parseClipboardText(text)!.groups.single.rows.single;
      expect(row.values, [10, 20]);
      expect(row.eases, hasLength(2));
      final firstIn = row.eases[0].$1 as BridgeSideInterp_Bezier;
      expect(firstIn.field0.speed, closeTo(12.5, 1e-9));
      expect(firstIn.field0.influence, closeTo(0.25, 1e-9));
      expect(row.eases[0].$2, isA<BridgeSideInterp_Hold>());
      expect(row.eases[1].$1, isA<BridgeSideInterp_Linear>());
      expect(row.eases[1].$2, isA<BridgeSideInterp_Bezier>());
    });

    /// A table from another editor has values and no easing columns; it must
    /// still paste, as linear keys, rather than being refused.
    test('a table with no easing columns still parses', () {
      const text = 'Some Editor 1.0 Keyframe Data\n'
          '\n'
          '\tUnits Per Second\t25\n'
          '\n'
          'Effects\tFL Depth Of Field #1\tfocal point #5\n'
          '\tFrame\t\n'
          '\t5\t208\t\n'
          '\n'
          'End of Keyframe Data\n';
      final parsed = parseClipboardText(text);
      expect(parsed, isNotNull);
      expect(parsed!.fps, 25);
      final row = parsed.groups.single.rows.single;
      expect(row.frame, 5);
      expect(row.values, [208]);
      expect(row.eases, isEmpty);
    });

    test('rejects text that is not keyframe data', () {
      expect(parseClipboardText('hello world'), isNull);
      expect(parseClipboardText(''), isNull);
    });
  });

  group('gridValues', () {
    test('a range that is not a range rules nothing', () {
      // Every one of these used to spin `powerOfTenUnder` for ever on the
      // interface's own thread: the application froze where it stood and never
      // drew another frame. A test that fails by never finishing is the honest
      // shape for that.
      expect(
          gridValues(double.negativeInfinity, double.infinity, 360), isEmpty);
      expect(gridValues(0, double.infinity, 360), isEmpty);
      expect(gridValues(double.negativeInfinity, 0, 360), isEmpty);
      expect(gridValues(double.nan, 1, 360), isEmpty);
      expect(gridValues(0, double.nan, 360), isEmpty);
    });
  });
}

// ---------------------------------------------------------------------------
// The Vegas speed envelope.
// ---------------------------------------------------------------------------

/// Tangent modes — Auto / Clamp / Free (docs/impl/keyframe-eval.md §6,
/// docs/impl/timeline-interaction.md §6.3). The numbers here are the ones
/// `crates/lumit-core/src/anim.rs`'s own tests assert, so the two ports are
/// held to one answer.
void tangentModeTests() {
  BridgeSideInterp auto({bool clamped = false}) => BridgeSideInterp.auto(
      BridgeAutoSide(clamped: clamped, speed: 0, influence: 1 / 3));

  group('automatic tangents', () {
    test('clamped, they do not overshoot a peak', () {
      List<BridgeKeyframe> peak(BridgeSideInterp side) => [
            key(0, 1, 0),
            key(1, 1, 10, interpIn: side, interpOut: side),
            key(2, 1, 9),
          ];
      final clamped = peak(auto(clamped: true));
      expect(autoSpeedAt(clamped, 1, clamped: true), 0);
      for (var i = 0; i <= 100; i++) {
        expect(evaluateKeys(clamped, i / 50), lessThanOrEqualTo(10 + 1e-9));
      }
      // The unclamped aim tilts uphill past the peak, and does overshoot.
      final smooth = peak(auto());
      expect(autoSpeedAt(smooth, 1, clamped: false), closeTo(4.5, 1e-12));
      var high = double.negativeInfinity;
      for (var i = 0; i <= 100; i++) {
        final v = evaluateKeys(smooth, i / 50);
        if (v > high) high = v;
      }
      expect(high, greaterThan(10 + 1e-6));
    });
  });

  group('tangent mode switching', () {
    test('Free → Auto → Free keeps the custom ease', () {
      const custom = BridgeSideInterp.bezier(
          BridgeBezierSide(speed: 7.25, influence: .82));
      expect(tangentModeOf(custom), TangentMode.free);
      final automatic = withTangentMode(custom, TangentMode.auto);
      expect(tangentModeOf(automatic), TangentMode.auto);
      final clamped = withTangentMode(automatic, TangentMode.clamp);
      expect(tangentModeOf(clamped), TangentMode.clamp);
      expect(withTangentMode(clamped, TangentMode.free), custom);
    });
  });
}

void envelopeTests() {
  group('the Vegas speed envelope', () {
    test('setting a speed re-integrates the frames after it, start pinned', () {
      final keys = [key(0, 1, 0.0), key(2, 1, 2.0), key(4, 1, 4.0)];
      // Drag the *first* point to 300%: the span to the second key now runs
      // at an average of (300 + 100) / 2 = 200%, advancing 4s of source in 2s.
      final out = setEnvelopeSpeed(keys, 0, 300);
      expect(out[0].value, closeTo(0.0, 1e-9), reason: 'the start is pinned');
      expect(out[1].value, closeTo(4.0, 1e-9));
      // …and everything past it carries the shift, the Vegas feel: the second
      // span still runs at 100% average, so it still advances 2s.
      expect(out[2].value, closeTo(6.0, 1e-9));
      // Every keyframe *time* stayed exactly put (the beat-sync covenant).
      expect([for (final k in out) rationalSeconds(k.time)], [0.0, 2.0, 4.0]);
    });

    // The claim the whole envelope rests on: it is not a simplified view of
    // the curve, it *is* the curve. If this fails, the two lenses disagree and
    // a ramp drawn in one reads wrong in the other.
    test('the curve under a straight envelope has an exactly linear speed', () {
      final keys = [key(0, 1, 0.0), key(4, 1, 4.0)];
      final ramped = setEnvelopeSpeed(keys, 1, 300); // 100% → 300%
      // Strictly inside the span: `evaluateKeysSpeed` is 0 *at* the first and
      // last key by the engine's own outside-the-keys rule, so the endpoints
      // are checked through the points themselves just below.
      for (var i = 1; i < 8; i++) {
        final t = i / 8 * 4.0;
        // The straight line between the two points, in source units.
        final expected = (100 + (300 - 100) * (t / 4.0)) / 100;
        expect(evaluateKeysSpeed(ramped, t), closeTo(expected, 1e-6),
            reason: 'speed at t=$t should sit on the envelope line');
      }
      expect(envelopeSpeeds(ramped), [closeTo(100, 1e-9), closeTo(300, 1e-9)]);
    });

    /// **The invariant the whole envelope rests on**, checked after the
    /// operation that used to break it.
    ///
    /// A key's stored tangent is a speed; its span's chord is an average. Move
    /// a key in time and the chord changes while the tangent does not, so a
    /// span that was straight stopped being straight — the curve bulged and
    /// the graph described playback the points did not say. It looked like it
    /// fixed itself, because the next speed drag re-ran the integration.
    test('moving a point in time leaves every span straight', () {
      final keys = [key(0, 1, 0.0), key(2, 1, 2.0), key(6, 1, 10.0)];
      final ramped = setEnvelopeSpeed(keys, 1, 250);
      final speedsBefore = envelopeSpeeds(ramped);

      // Drag the middle point later, which is what bent it.
      final moved = moveEnvelopePoint(ramped, 1, rat(3, 1));

      expect(rationalSeconds(moved[1].time), closeTo(3.0, 1e-9));
      expect(
          envelopeSpeeds(moved),
          [
            closeTo(speedsBefore[0], 1e-6),
            closeTo(speedsBefore[1], 1e-6),
            closeTo(speedsBefore[2], 1e-6),
          ],
          reason: 'the point keeps the speed it had');

      // And every span reads as the straight line between its two points —
      // sampled inside each one, which is where a bulge would show.
      for (var span = 0; span + 1 < moved.length; span++) {
        final t0 = rationalSeconds(moved[span].time);
        final t1 = rationalSeconds(moved[span + 1].time);
        final v0 = envelopeSpeeds(moved)[span];
        final v1 = envelopeSpeeds(moved)[span + 1];
        for (var i = 1; i < 8; i++) {
          final f = i / 8;
          final t = t0 + (t1 - t0) * f;
          expect(evaluateKeysSpeed(moved, t) * 100,
              closeTo(v0 + (v1 - v0) * f, 1e-4),
              reason: 'span \$span at t=\$t sits on its own straight line');
        }
      }
    });
  });
}

void envelopeShapeTests() {
}
