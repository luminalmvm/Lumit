// The text animator helpers both panels share: modes, bounce presets, the
// rows a selector shows, renaming and moving animators, the Stagger copies.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/text_animator_rows_frb.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

void main() {
  test('every Mode survives the trip through invert and combine', () {
    for (final m in SelectorModeChoice.values) {
      expect(selectorModeOf(selectorWithMode(freshSelector(), m)), m);
    }
    expect(selectorModesFor(0),
        [SelectorModeChoice.add, SelectorModeChoice.subtract]);
  });

  test('a bounce preset sets the three numbers and is read back', () {
    for (final p in [BouncePreset.soft, BouncePreset.medium, BouncePreset.elastic]) {
      expect(bouncePresetOf(selectorWithBounce(freshSelector(), p)), p);
    }
    expect(bouncePresetOf(freshSelector()), BouncePreset.none);
  });

  test('Range rows fold Based on, Mode, Shape and the eases under Advanced',
      () {
    final items = selectorItemsOf(freshSelector(), 0);
    final open = [
      for (final i in items)
        if (!i.advanced)
          switch (i) {
            SelectorValueItem(:final value) => value.name,
            SelectorSettingItem(:final setting) => setting.name,
          },
    ];
    expect(open, ['selector', 'rangeStart', 'rangeEnd', 'rangeOffset']);
    // Seed only with Randomize order.
    expect(
        items.whereType<SelectorSettingItem>().map((i) => i.setting),
        isNot(contains(AnimatorSetting.seed)));
    expect(
        selectorItemsOf(selectorWith(freshSelector(), randomize: true), 0)
            .whereType<SelectorSettingItem>()
            .map((i) => i.setting),
        contains(AnimatorSetting.seed));
  });

  test('renaming an animator carries the links to it', () {
    final a = freshTextAnimator('Arrive');
    final b = animatorWith(freshTextAnimator('Fade'), follow: 'Arrive');
    final out = animatorsRenamed([a, b], 0, 'Drop');
    expect(out.map((x) => x.name), ['Drop', 'Fade']);
    expect(out[1].follow, 'Drop');
    expect(animatorsMoved(out, 1, 0).map((x) => x.name), ['Fade', 'Drop']);
  });

  test('animators are followed by name through a move, a removal and a rename',
      () {
    final a = freshTextAnimator('A');
    final b = freshTextAnimator('B');
    final c = freshTextAnimator('C');
    expect(animatorMoves([a, b, c], [c, a, b]), [1, 2, 0]);
    expect(animatorMoves([a, b, c], [a, c]), [0, null, 1]);
    expect(animatorMoves([a, b], [a, animatorWith(b, name: 'Wave')]), [0, 1]);
    // Two of one name, as a preset lays: the first to the first.
    expect(animatorMoves([a, a, b], [b, a, a]), [1, 2, 0]);
    expect(movedAnimatorKey('L/animators/2/advanced', 'L/animators/', [1, 2, 0]),
        'L/animators/0/advanced');
    expect(movedAnimatorKey('L/animators/1', 'L/animators/', [0, null, 1]), isNull);
    expect(movedAnimatorKey('L/transform', 'L/animators/', [1, 0]), 'L/transform');
  });

  test('a selector taken out moves the rows, Advanced and keys after it up',
      () {
    final three = animatorWith(freshTextAnimator('A'), selectors: [
      freshSelector(BridgeSelectorKind.stagger),
      freshSelector(BridgeSelectorKind.wiggly),
    ]);
    final two = animatorWithoutSelector(three, 1);
    final sm = selectorMovesOf(three, two);
    expect(sm, [0, null, 1]);
    const stem = 'L/animators/';
    final sel = {0: sm};
    String? tl(String k) =>
        followedAnimatorKey(k, stem, [0], sel, timeline: true);
    expect(tl('L/animators/0/s2/maxAmount#2'), 'L/animators/0/s1/maxAmount#2');
    expect(tl('L/animators/0/s2/advanced'), 'L/animators/0/s1/advanced');
    expect(tl('L/animators/0/s1/staggerAmount'), isNull);
    expect(tl('L/animators/0/rangeStart'), 'L/animators/0/rangeStart');
    expect(tl('L/animators/0/opacity'), 'L/animators/0/opacity');
    expect(followedAnimatorKey('L/0/2', 'L/', [0], sel), 'L/0/1');
    expect(followedAnimatorKey('L/0/1', 'L/', [0], sel), isNull);
    expect(followedAnimatorKey('L/0', 'L/', [0], sel), 'L/0');
  });

  test('a rename never takes another animator''s name', () {
    final out = animatorsRenamed(
        [freshTextAnimator('A'), freshTextAnimator('B')], 1, 'A');
    expect(out.map((x) => x.name), ['A', 'A 2']);
  });

  test('a pasted pair stays linked under its new names', () {
    final arrive = freshTextAnimator('Arrive');
    final fade = animatorWith(freshTextAnimator('Fade'), follow: 'Arrive');
    final out = animatorsPasted([arrive], [arrive, fade]);
    expect(out.map((x) => x.name), ['Arrive', 'Arrive 2', 'Fade']);
    expect(out[2].follow, 'Arrive 2');
    // A copy linked to one not copied keeps its link.
    expect(animatorsPasted([arrive], [fade]).last.follow, 'Arrive');
    // A preset laying two of one name, put on twice: the pair keeps one
    // name, free of the first pair's.
    final tracking = freshTextAnimator('Tracking in');
    expect(
        animatorsPasted([tracking, tracking], [tracking, tracking])
            .map((x) => x.name),
        ['Tracking in', 'Tracking in', 'Tracking in 2', 'Tracking in 2']);
  });

  test('a Stagger with Total duration spreads over Delay whatever the length',
      () {
    final s = selectorWith(freshSelector(BridgeSelectorKind.stagger),
        delay: const BridgeScalar.static_(1), total: true);
    for (final n in [3, 11]) {
      final offsets = staggerOffsets(s, n, 0);
      expect(offsets.reduce((a, b) => a > b ? a : b), closeTo(1, 1e-9));
    }
    expect(unitCount('ab c\nd', BridgeSelectorBasis.lines), 2);
    expect(unitCount('ab c', BridgeSelectorBasis.charactersExcludingSpaces), 3);
  });
}