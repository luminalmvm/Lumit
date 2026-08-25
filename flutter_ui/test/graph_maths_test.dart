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
  group('evaluateKeys', () {
    test('clamps past the ends and lerps a straight span', () {
      final keys = [key(0, 1, 10), key(1, 1, 20)];
      expect(evaluateKeys(keys, -1), 10);
      expect(evaluateKeys(keys, 2), 20);
      expect(evaluateKeys(keys, 0.5), closeTo(15, 1e-12));
    });

    test('hold-out wins its span', () {
      final keys = [
        key(0, 1, 10, interpOut: const BridgeSideInterp.hold()),
        key(1, 1, 20),
      ];
      expect(evaluateKeys(keys, 0.999), 10);
      expect(evaluateKeys(keys, 1), 20);
    });

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

    test('a linear side inside a bezier span lies on the chord', () {
      // Out side linear, in side eased: at the linear end the curve leaves at
      // the chord slope (100 units/s over the span).
      final keys = [
        key(0, 1, 0),
        key(1, 1, 100, interpIn: easyEase),
      ];
      final nearStart = evaluateKeys(keys, 0.01);
      expect(nearStart, closeTo(1.0, 0.25),
          reason: 'leaves the first key at roughly the chord slope');
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

  group('evaluateKeysSpeed', () {
    test('is the chord on straight spans, zero outside and across holds', () {
      final keys = [
        key(0, 1, 0),
        key(1, 1, 10),
        key(2, 1, 10, interpIn: const BridgeSideInterp.linear())
      ];
      expect(evaluateKeysSpeed(keys, -1), 0);
      expect(evaluateKeysSpeed(keys, 0.5), closeTo(10, 1e-9));
      expect(evaluateKeysSpeed(keys, 3), 0);
    });

    test('an eased span is flat at its keys and fastest in the middle', () {
      final keys = [
        key(0, 1, 0, interpOut: easyEase),
        key(1, 1, 100, interpIn: easyEase),
      ];
      expect(evaluateKeysSpeed(keys, 0.001), lessThan(1));
      expect(evaluateKeysSpeed(keys, 0.5), greaterThan(100));
      expect(evaluateKeysSpeed(keys, 0.999), lessThan(1));
    });

    test('sideSpeedAtKey reads the side parameter directly', () {
      const fast =
          BridgeSideInterp.bezier(BridgeBezierSide(speed: 42, influence: 0.5));
      final keys = [
        key(0, 1, 0, interpOut: fast),
        key(1, 1, 100, interpIn: easyEase),
      ];
      expect(sideSpeedAtKey(keys, 0, isOut: true), 42);
      expect(sideSpeedAtKey(keys, 1, isOut: false), 0);
      expect(sideSpeedAtKey(keys, 0, isOut: false), 0,
          reason: 'no neighbour on that side');
      // A linear side reads the chord.
      final straight = [key(0, 1, 0), key(1, 1, 100)];
      expect(sideSpeedAtKey(straight, 0, isOut: true), closeTo(100, 1e-9));
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

    test('a drag past the span clamps its influence to 1', () {
      final r = handleFromDrag(
        keyTime: 0,
        keyValue: 0,
        neighbourTime: 1,
        isOut: true,
        dragTime: 5,
        dragValue: 0,
      );
      expect(r.influence, 1);
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

    test('a static value alone still yields a usable range', () {
      final (lo, hi) = fitValueRange([], [50]);
      expect(lo, lessThan(50));
      expect(hi, greaterThan(50));
    });

    test('the speed range always includes zero', () {
      final keys = [key(0, 1, 0), key(1, 1, 100)];
      final (lo, hi) = fitSpeedRange([keys]);
      expect(lo, lessThanOrEqualTo(0));
      expect(hi, greaterThanOrEqualTo(100));
    });
  });

  group('keyframe clipboard text', () {
    test('writes the named table and parses itself back', () {
      final text = lumitClipboardText(
        version: '0.1.0',
        fps: 25,
        width: 1920,
        height: 816,
        groups: [
          const LumitClipGroup(
            property: ['Transform', 'Position'],
            columns: ['X pixels', 'Y pixels'],
            rows: [
              LumitClipRow(frame: 0, values: [100, 200]),
              LumitClipRow(frame: 5, values: [300, 412.5]),
            ],
          ),
          const LumitClipGroup(
            property: ['Effects', 'Gaussian blur', 'Radius'],
            columns: ['Value'],
            rows: [
              LumitClipRow(frame: 5, values: [208]),
            ],
          ),
        ],
      );
      expect(text, startsWith('Lumit 0.1.0 Keyframe Data'));
      expect(text, isNot(contains('After Effects')));
      expect(text, contains('\tUnits Per Second\t25'));
      expect(text, contains('Transform\tPosition'));
      expect(text, contains('\t5\t300\t412.5\t'));
      expect(text, contains('End of Keyframe Data'));

      final parsed = parseClipboardText(text);
      expect(parsed, isNotNull);
      expect(parsed!.fps, 25);
      expect(parsed.groups, hasLength(2));
      expect(parsed.groups.first.property, ['Transform', 'Position']);
      expect(parsed.groups.first.columns, ['X pixels', 'Y pixels']);
      expect(parsed.groups.first.rows[1].frame, 5);
      expect(parsed.groups.first.rows[1].values, [300, 412.5]);
      expect(parsed.groups[1].rows.single.values, [208]);
    });

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
}

// ---------------------------------------------------------------------------
// The Vegas speed envelope (K-247).
// ---------------------------------------------------------------------------

void envelopeTests() {
  group('the Vegas speed envelope', () {
    test('an identity retime reads as 100% throughout', () {
      // Ten seconds of layer time showing ten seconds of source.
      final keys = [key(0, 1, 0.0), key(10, 1, 10.0)];
      expect(envelopeSpeeds(keys), everyElement(closeTo(100, 1e-9)));
    });

    test('half speed reads as 50%', () {
      final keys = [key(0, 1, 0.0), key(10, 1, 5.0)];
      expect(envelopeSpeeds(keys), everyElement(closeTo(50, 1e-9)));
    });

    test('a backwards run reads negative', () {
      final keys = [key(0, 1, 5.0), key(5, 1, 0.0)];
      expect(envelopeSpeeds(keys), everyElement(closeTo(-100, 1e-9)));
    });

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

    test('a speed set is the speed read back', () {
      final keys = [key(0, 1, 0.0), key(2, 1, 2.0), key(4, 1, 4.0)];
      final out = setEnvelopeSpeed(keys, 1, 250);
      expect(envelopeSpeeds(out)[1], closeTo(250, 1e-9));
      expect(envelopeSpeeds(out)[0], closeTo(100, 1e-9),
          reason: 'the points either side are untouched');
      expect(envelopeSpeeds(out)[2], closeTo(100, 1e-9));
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

    test('the default range is 125 down to -25, and only ever grows', () {
      final flat = [key(0, 1, 0.0), key(4, 1, 4.0)];
      expect(fitEnvelopeRange([flat]), envelopeDefaultRange,
          reason: 'an ordinary clip opens at exactly the documented range');

      final fast = setEnvelopeSpeed(flat, 1, 850);
      final (lo, hi) = fitEnvelopeRange([fast]);
      expect(hi, greaterThan(850),
          reason: 'a fast ramp is framed, not clipped');
      expect(lo, -25.0, reason: 'the floor did not move');
      // The headroom above normal playback is the point of the top figure
      // (K-250): a flat 100% line must not sit on the graph's own edge.
      expect(envelopeDefaultRange.$2, 125.0);

      final reversed = setEnvelopeSpeed(flat, 1, -400);
      final (rlo, rhi) = fitEnvelopeRange([reversed]);
      expect(rlo, lessThan(-400));
      expect(rhi, 125.0, reason: 'the ceiling did not move either');
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
      expect(envelopeSpeeds(moved), [
        closeTo(speedsBefore[0], 1e-6),
        closeTo(speedsBefore[1], 1e-6),
        closeTo(speedsBefore[2], 1e-6),
      ], reason: 'the point keeps the speed it had');

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
          expect(evaluateKeysSpeed(moved, t) * 100, closeTo(v0 + (v1 - v0) * f, 1e-4),
              reason: 'span \$span at t=\$t sits on its own straight line');
        }
      }
    });

    test('an empty or mismatched envelope leaves the keys alone', () {
      expect(envelopeToKeys(const [], const []), isEmpty);
      final keys = [key(0, 1, 0.0), key(4, 1, 4.0)];
      expect(envelopeToKeys(keys, const [100.0]), same(keys));
      expect(setEnvelopeSpeed(keys, 9, 200), same(keys));
    });
  });
}

void envelopeShapeTests() {
  group('the envelope leaves untouched keys alone', () {
    test('a flat envelope is all linear sides', () {
      final keys = [key(0, 1, 0.0), key(2, 1, 2.0), key(4, 1, 4.0)];
      final out = envelopeToKeys(keys, const [100.0, 100.0, 100.0]);
      for (final k in out) {
        expect(k.interpIn, isA<BridgeSideInterp_Linear>());
        expect(k.interpOut, isA<BridgeSideInterp_Linear>());
      }
    });

    // The bug: dragging one point re-shaped every key on the channel, so keys
    // nobody touched changed glyph from a diamond to a circle.
    test('a key whose speed is still its chord keeps a linear side', () {
      final keys = [
        key(0, 1, 0.0),
        key(2, 1, 2.0),
        key(4, 1, 4.0),
        key(6, 1, 6.0),
      ];
      // Ramp only the middle span: keys 0 and 3 are nowhere near it.
      final out = envelopeToKeys(keys, const [100.0, 100.0, 300.0, 300.0]);
      expect(out[0].interpOut, isA<BridgeSideInterp_Linear>(),
          reason: 'the first span is still a flat 100%');
      expect(out[3].interpIn, isA<BridgeSideInterp_Linear>(),
          reason: 'and so is the last');
      expect(out[1].interpOut, isA<BridgeSideInterp_Bezier>(),
          reason: 'the ramped span genuinely leaves at a non-chord speed');
      expect(out[2].interpIn, isA<BridgeSideInterp_Bezier>());
    });

    test('flattening a ramp puts the linear sides back', () {
      final keys = [key(0, 1, 0.0), key(2, 1, 2.0), key(4, 1, 4.0)];
      final ramped = setEnvelopeSpeed(keys, 1, 300);
      final flat = setEnvelopeSpeed(ramped, 1, 100);
      for (final k in flat) {
        expect(k.interpIn, isA<BridgeSideInterp_Linear>());
        expect(k.interpOut, isA<BridgeSideInterp_Linear>());
      }
      expect(envelopeSpeeds(flat), everyElement(closeTo(100, 1e-9)));
    });

    test('the speeds still read back after the linear-side tidying', () {
      final keys = [key(0, 1, 0.0), key(2, 1, 2.0), key(4, 1, 4.0)];
      final out = envelopeToKeys(keys, const [100.0, 300.0, 50.0]);
      expect(envelopeSpeeds(out), [
        closeTo(100, 1e-9),
        closeTo(300, 1e-9),
        closeTo(50, 1e-9),
      ]);
    });
  });
}
