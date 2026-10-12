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
        sourceIn: const BridgeRational(num: 0, den: 1),
        sourceOut: const BridgeRational(num: 1, den: 1),
        sourceIsComp: false,
      );

  group('the shapes', () {
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
