// A clip's fade curves and where they run (docs/impl/audio-timeline.md §6,
// plan 1): every shape starts at silence and ends at full level and never turns
// back, Fast against Fast keeps a crossfade as loud as either clip was, and the
// power complement of Fast is Fast. Pure, so the maths is checked here rather
// than by looking at a lane.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/clip_fades.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

void main() {
  /// A clip on the comp's clock, with the fades it carries.
  BridgeClip clip({
    required int start,
    required int end,
    double fadeIn = 0,
    double fadeOut = 0,
    BridgeClipFadeShape shape = const BridgeClipFadeShape.fast(),
  }) =>
      BridgeClip(
        id: UuidValue.fromString(const Uuid().v4()),
        placeStart: const BridgeRational(num: 0, den: 1),
        placeDuration: const BridgeRational(num: 1, den: 1),
        gainDb: 0,
        startFrame: start,
        endFrame: end,
        retimed: false,
        retime: const BridgeScalar.static_(0),
        fadeIn: BridgeClipFade(seconds: fadeIn, shape: shape),
        fadeOut: BridgeClipFade(seconds: fadeOut, shape: shape),
        effects: const [],
        fx: true,
        sourceName: 'tone',
      );

  group('the shapes', () {
    test('every shape runs from silence to full level and never turns back',
        () {
      for (final preset in clipFadeShapes) {
        expect(clipFadeGain(preset.shape, 0), closeTo(0, 1e-9),
            reason: '${preset.id} does not start at silence');
        expect(clipFadeGain(preset.shape, 1), closeTo(1, 1e-9),
            reason: '${preset.id} does not reach full level');
        var last = 0.0;
        for (var i = 1; i <= 64; i++) {
          final gain = clipFadeGain(preset.shape, i / 64);
          expect(gain, greaterThanOrEqualTo(last - 1e-9),
              reason: '${preset.id} dips at ${i / 64}');
          last = gain;
        }
      }
    });

    test('a Custom shape is the curve its four numbers draw', () {
      const shape =
          BridgeClipFadeShape.custom(x1: 1 / 3, y1: 0, x2: 2 / 3, y2: 1);
      expect(clipFadeGain(shape, 0), closeTo(0, 1e-9));
      expect(clipFadeGain(shape, 1), closeTo(1, 1e-9));
      // Eased at both ends, so it is behind the straight line early on.
      expect(clipFadeGain(shape, 0.25), lessThan(0.25));
      expect(clipFadeGain(shape, 0.5), closeTo(0.5, 1e-6));
    });

    test('Fast against Fast sums its squares to one across a crossfade', () {
      const fast = BridgeClipFadeShape.fast();
      for (var i = 0; i <= 32; i++) {
        final u = i / 32;
        final rising = clipFadeGain(fast, u);
        final falling = clipFadeGain(fast, 1 - u);
        expect(rising * rising + falling * falling, closeTo(1, 1e-9),
            reason: 'the join changes level at $u');
      }
    });

    test('the power complement of Fast is Fast', () {
      const fast = BridgeClipFadeShape.fast();
      for (var i = 0; i <= 32; i++) {
        final u = i / 32;
        expect(keepLevelGain(fast, u), closeTo(clipFadeGain(fast, u), 1e-9));
      }
    });

    test('a complement keeps the level whatever curve it answers', () {
      const shape = BridgeClipFadeShape.slow();
      for (var i = 0; i <= 32; i++) {
        final u = i / 32;
        final rising = keepLevelGain(shape, u);
        final falling = clipFadeGain(shape, 1 - u);
        expect(rising * rising + falling * falling, closeTo(1, 1e-9));
      }
    });

    test('a fitted curve draws the shape it was fitted to', () {
      const fast = BridgeClipFadeShape.fast();
      final fitted =
          customFadeShape(fitFadeCurve((u) => clipFadeGain(fast, u)));
      for (var i = 0; i <= 16; i++) {
        final u = i / 16;
        expect(clipFadeGain(fitted, u), closeTo(clipFadeGain(fast, u), 0.01));
      }
    });
  });

  group('where a fade runs', () {
    test('a lone fade is its own seconds, clamped to the clip', () {
      final short = clip(start: 0, end: 10, fadeIn: 100);
      expect(clipFadeEdge(short, into: true, fps: 25), 10,
          reason: 'a fade longer than the clip stops at the far end');
      final one = clip(start: 4, end: 54, fadeIn: 1, fadeOut: 2);
      expect(clipFadeEdge(one, into: true, fps: 25), 29);
      expect(clipFadeEdge(one, into: false, fps: 25), 4);
    });

    test('a corner drag maps to seconds and back', () {
      final one = clip(start: 0, end: 100, fadeIn: 1);
      // A drag in flight is read where the pointer left it, not where the
      // document still says the fade ends.
      final live = (clip: one.id.toString(), into: true, seconds: 2.0);
      expect(clipFadeEdge(one, into: true, fps: 25, live: live), 50);
      expect(clipFadeSeconds(one, into: true, live: live), 2);
      expect(clipFadeSeconds(one, into: false, live: live), 0,
          reason: 'the other end of the clip is not being dragged');
    });

    test('a track of two clips draws each lone fade', () {
      final clips = [
        clip(start: 0, end: 50, fadeIn: 1),
        clip(start: 60, end: 100, fadeOut: 1),
      ];
      final ramps = clipFadeRamps(clips, 25);
      expect(ramps.length, 2);
      expect(ramps.first.into, isTrue);
      expect(ramps.first.from, 0);
      expect(ramps.first.to, 25);
      expect(ramps.last.into, isFalse);
      expect(ramps.last.from, 75);
      expect(ramps.last.to, 100);
    });

    test('an overlap is the crossfade, drawn once for the pair', () {
      final outgoing = clip(start: 0, end: 60, fadeOut: 1);
      final incoming = clip(start: 40, end: 100, fadeIn: 1);
      final clips = [outgoing, incoming];
      expect(clipFadePartner(clips, incoming, into: true)?.id, outgoing.id);
      expect(clipFadePartner(clips, outgoing, into: false)?.id, incoming.id);

      final ramps = clipFadeRamps(clips, 25);
      expect(ramps.length, 2, reason: 'the pair is drawn once, not twice');
      for (final ramp in ramps) {
        expect(ramp.from, 40);
        expect(ramp.to, 60,
            reason: 'the overlap is the length, whatever the seconds say');
      }
      expect(ramps.map((ramp) => ramp.into), containsAll([true, false]));
    });

    test('a right click knows which fade it landed on', () {
      final outgoing = clip(start: 0, end: 60, fadeOut: 1);
      final incoming = clip(start: 40, end: 100, fadeIn: 1);
      final clips = [outgoing, incoming];
      expect(clipFadeSideAt(clips, incoming, 50, 25), isTrue,
          reason: 'anywhere in an overlap is that fade');
      expect(clipFadeSideAt(clips, incoming, 80, 25), isNull,
          reason: 'the plain body is the clip, not a fade');

      final lone = clip(start: 0, end: 100, fadeIn: 1, fadeOut: 1);
      expect(clipFadeSideAt([lone], lone, 10, 25), isTrue);
      expect(clipFadeSideAt([lone], lone, 90, 25), isFalse);
      expect(clipFadeSideAt([lone], lone, 50, 25), isNull);

      final bare = clip(start: 0, end: 100);
      expect(clipFadeSideAt([bare], bare, 0, 25), isNull,
          reason: 'a clip with no fade offers none to shape');
    });
  });
}
