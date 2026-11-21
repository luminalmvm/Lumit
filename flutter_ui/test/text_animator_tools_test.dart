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
