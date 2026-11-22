// What a text animator's rows share between the Effect controls panel and
// the Timeline: its selectors (more than one, each with its Mode), the rows
// each selector shows and which of them fold away under Advanced, the choice
// controls, copy and paste, the user's own presets, and the small curve
// beside Shape, the eases and a Stagger's Amount.
//
// The weights themselves are the engine's; the curve preview only sketches
// the same formula so the row can show it without a bridge call per frame.

import 'dart:convert';
import 'dart:io';
import 'dart:math' as math;

import 'package:flutter/gestures.dart' show kDoubleTapTimeout;
import 'package:flutter/scheduler.dart' show SchedulerBinding, SchedulerPhase;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../icons/icons.dart' show iconSize;
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/theme_name_dialog.dart' show askThemeName;
import '../widgets/controls.dart';
import '../widgets/layer_picker.dart';
import 'graph_maths.dart' show evaluateScalar, keysOf, rationalSeconds;
import '../state/font_axes.dart' show fontAxesReady;
import 'fx_section.dart'
    show FxRenameField, fxKeyColumnWidth, fxRowHeight, fxTwoColumnRow;
import 'text_animation_presets.dart' show reversedAnimator, reversedScalar;
import 'timeline_extras_frb.dart' show showMenuAt;
import 'timeline_mask_rows_frb.dart' show RowButton;
import 'layer_fold_frb.dart' show animatorsPath;
import 'text_animator_rows_frb.dart';

// ---- Selectors ------------------------------------------------------------

/// A fresh selector of [kind]: every number where it changes nothing.
BridgeRangeSelector freshSelector(
        [BridgeSelectorKind kind = BridgeSelectorKind.range]) =>
    BridgeRangeSelector(
      start: const BridgeScalar.static_(0),
      end: const BridgeScalar.static_(100),
      offset: const BridgeScalar.static_(0),
      basis: BridgeSelectorBasis.characters,
      // A Proximity selector starts Smooth, as [animatorWithSelectorKind].
      shape: kind == BridgeSelectorKind.proximity
          ? BridgeSelectorShape.smooth
          : BridgeSelectorShape.square,
      mode: BridgeSelectorMode.range,
      ease: 0,
      kind: kind,
      radius: const BridgeScalar.static_(200),
      invert: false,
      easeHigh: const BridgeScalar.static_(0),
      easeLow: const BridgeScalar.static_(0),
      amount: const BridgeScalar.static_(100),
      delay: const BridgeScalar.static_(0.1),
      delayEase: const BridgeScalar.static_(0),
      order: BridgeStaggerOrder.forward,
      bounceAmplitude: const BridgeScalar.static_(0),
      bounceFrequency: const BridgeScalar.static_(2),
      bounceDecay: const BridgeScalar.static_(3),
      minAmount: const BridgeScalar.static_(0),
      maxAmount: const BridgeScalar.static_(100),
      wiggles: const BridgeScalar.static_(2),
      correlation: const BridgeScalar.static_(50),
      seed: 0,
      combine: BridgeSelectorCombine.add,
      units: BridgeSelectorUnits.percentage,
      randomize: false,
      delayFrames: false,
      total: false,
      repeat: BridgeStaggerRepeat.once,
      bounceLast: false,
    );

/// How many selectors [a] has, the first counted.
int selectorCount(BridgeTextAnimator a) => 1 + a.selectors.length;

/// [a]'s selector [k], the first being 0.
BridgeRangeSelector selectorAt(BridgeTextAnimator a, int k) =>
    k <= 0 || k > a.selectors.length ? a.selector : a.selectors[k - 1];

/// [a] with its selector [k] replaced.
BridgeTextAnimator animatorWithSelectorAt(
        BridgeTextAnimator a, int k, BridgeRangeSelector s) =>
    k <= 0
        ? animatorWith(a, selector: s)
        : animatorWith(a, selectors: [
            for (var i = 0; i < a.selectors.length; i++)
              if (i == k - 1) s else a.selectors[i],
          ]);

/// [a] without its selector [k]; the first one stays, since an animator is
/// never without a selector.
BridgeTextAnimator animatorWithoutSelector(BridgeTextAnimator a, int k) =>
    k <= 0
        ? a
        : animatorWith(a, selectors: [
            for (var i = 0; i < a.selectors.length; i++)
              if (i != k - 1) a.selectors[i],
          ]);

/// [s] made [kind], Square going Smooth for a Proximity selector.
BridgeRangeSelector selectorWithKind(
        BridgeRangeSelector s, BridgeSelectorKind kind) =>
    selectorWith(s,
        kind: kind,
        shape: kind == BridgeSelectorKind.proximity &&
                s.shape == BridgeSelectorShape.square
            ? BridgeSelectorShape.smooth
            : null);

/// A selector's Mode, as After Effects lists it: how it is put together with
/// the selectors above it. Subtract is the selector's `invert`.
enum SelectorModeChoice { add, subtract, intersect, min, max, difference }

SelectorModeChoice selectorModeOf(BridgeRangeSelector s) => s.invert
    ? SelectorModeChoice.subtract
    : switch (s.combine) {
        BridgeSelectorCombine.add => SelectorModeChoice.add,
        BridgeSelectorCombine.intersect => SelectorModeChoice.intersect,
        BridgeSelectorCombine.min => SelectorModeChoice.min,
        BridgeSelectorCombine.max => SelectorModeChoice.max,
        BridgeSelectorCombine.difference => SelectorModeChoice.difference,
      };

BridgeRangeSelector selectorWithMode(
        BridgeRangeSelector s, SelectorModeChoice m) =>
    selectorWith(s,
        invert: m == SelectorModeChoice.subtract,
        combine: switch (m) {
          SelectorModeChoice.intersect => BridgeSelectorCombine.intersect,
          SelectorModeChoice.min => BridgeSelectorCombine.min,
          SelectorModeChoice.max => BridgeSelectorCombine.max,
          SelectorModeChoice.difference => BridgeSelectorCombine.difference,
          _ => BridgeSelectorCombine.add,
        });

String selectorModeLabel(SelectorModeChoice m) => switch (m) {
      SelectorModeChoice.add => l10n.textAnimatorModeAdd,
      SelectorModeChoice.subtract => l10n.textAnimatorModeSubtract,
      SelectorModeChoice.intersect => l10n.textAnimatorModeIntersect,
      SelectorModeChoice.min => l10n.textAnimatorModeMin,
      SelectorModeChoice.max => l10n.textAnimatorModeMax,
      SelectorModeChoice.difference => l10n.textAnimatorModeDifference,
    };

/// The modes selector [k] offers: the first has nothing above it to meet,
/// so only Add and Subtract mean anything there.
List<SelectorModeChoice> selectorModesFor(int k) => k == 0
    ? const [SelectorModeChoice.add, SelectorModeChoice.subtract]
    : SelectorModeChoice.values;

String selectorBasisLabel(BridgeSelectorBasis b) => switch (b) {
      BridgeSelectorBasis.characters => l10n.textAnimatorBasisCharacters,
      BridgeSelectorBasis.charactersExcludingSpaces =>
        l10n.textAnimatorBasisCharactersExcludingSpaces,
      BridgeSelectorBasis.words => l10n.textAnimatorBasisWords,
      BridgeSelectorBasis.lines => l10n.textAnimatorBasisLines,
    };

String selectorShapeLabel(BridgeSelectorShape s) => switch (s) {
      BridgeSelectorShape.square => l10n.textAnimatorShapeSquare,
      BridgeSelectorShape.ramp => l10n.textAnimatorShapeRamp,
      BridgeSelectorShape.smooth => l10n.textAnimatorShapeSmooth,
      BridgeSelectorShape.bounce => l10n.textAnimatorShapeBounce,
    };

String selectorKindLabel(BridgeSelectorKind k) => switch (k) {
      BridgeSelectorKind.range => l10n.textAnimatorSelectorRange,
      BridgeSelectorKind.proximity => l10n.textAnimatorSelectorProximity,
      BridgeSelectorKind.wiggly => l10n.textAnimatorSelectorWiggly,
      BridgeSelectorKind.stagger => l10n.textAnimatorSelectorStagger,
    };

// ---- Bounce presets -------------------------------------------------------

/// A Stagger's bounce in one pick: amplitude, frequency and decay.
enum BouncePreset {
  none(0, 2, 3),
  soft(2, 2, 6),
  medium(5, 2.5, 4),
  elastic(12, 4, 2),
  custom(0, 0, 0);

  final double amplitude, frequency, decay;
  const BouncePreset(this.amplitude, this.frequency, this.decay);

  String get label => switch (this) {
        none => l10n.textAnimatorBounceNone,
        soft => l10n.textAnimatorBounceSoft,
        medium => l10n.textAnimatorBounceMedium,
        elastic => l10n.textAnimatorBounceElastic,
        custom => l10n.textAnimatorBounceCustom,
      };
}

/// The preset [s]'s three bounce numbers make, Custom when they are keyed or
/// match none. Any amplitude of 0 is None.
BouncePreset bouncePresetOf(BridgeRangeSelector s) {
  double? still(BridgeScalar v) =>
      v is BridgeScalar_Static ? v.field0 : null;
  final (a, f, d) = (
    still(s.bounceAmplitude),
    still(s.bounceFrequency),
    still(s.bounceDecay)
  );
  if (a == 0) return BouncePreset.none;
  for (final p in BouncePreset.values) {
    if (p != BouncePreset.custom &&
        p.amplitude == a &&
        p.frequency == f &&
        p.decay == d) {
      return p;
    }
  }
  return BouncePreset.custom;
}

BridgeRangeSelector selectorWithBounce(
        BridgeRangeSelector s, BouncePreset p) =>
    p == BouncePreset.custom
        ? s
        : selectorWith(s,
            bounceAmplitude: BridgeScalar.static_(p.amplitude),
            bounceFrequency: BridgeScalar.static_(p.frequency),
            bounceDecay: BridgeScalar.static_(p.decay));

// ---- A selector's rows ----------------------------------------------------

/// A text animator's selector choices, which have no curve.
enum AnimatorSetting {
  selector,
  target,
  invert,
  mode,
  units,
  basis,
  shape,
  randomize,
  seed,
  order,
  delayUnit,
  total,
  repeat,
  bouncePreset,
  bounceLast,
}

String animatorSettingLabel(AnimatorSetting setting) => switch (setting) {
      AnimatorSetting.selector => l10n.textAnimatorSelector,
      AnimatorSetting.target => l10n.textAnimatorTarget,
      AnimatorSetting.invert => l10n.textAnimatorInvert,
      AnimatorSetting.mode => l10n.textAnimatorMode,
      AnimatorSetting.units => l10n.textAnimatorUnits,
      AnimatorSetting.basis => l10n.textAnimatorBasis,
      AnimatorSetting.shape => l10n.textAnimatorShape,
      AnimatorSetting.randomize => l10n.textAnimatorRandomize,
      AnimatorSetting.seed => l10n.textAnimatorSeed,
      AnimatorSetting.order => l10n.textAnimatorOrder,
      AnimatorSetting.delayUnit => l10n.textAnimatorDelayIn,
      AnimatorSetting.total => l10n.textAnimatorTotal,
      AnimatorSetting.repeat => l10n.textAnimatorRepeat,
      AnimatorSetting.bouncePreset => l10n.textAnimatorBouncePreset,
      AnimatorSetting.bounceLast => l10n.textAnimatorBounceOn,
    };

/// One row of a selector: a choice or a number, and whether it folds away
/// under Advanced.
sealed class SelectorItem {
  final bool advanced;
  const SelectorItem(this.advanced);
}

final class SelectorSettingItem extends SelectorItem {
  final AnimatorSetting setting;
  const SelectorSettingItem(this.setting, [super.advanced = false]);
}

final class SelectorValueItem extends SelectorItem {
  final TextAnimatorValue value;
  const SelectorValueItem(this.value, [super.advanced = false]);
}

/// The rows selector [k] shows, in order. A Range's Start, End and Offset
/// stay open; Based on, Mode, Shape, the eases, Units and Randomize fold
/// under Advanced. Rows that would do nothing are left out: Seed only with a
/// random order, Mode only where there is something to meet.
List<SelectorItem> selectorItemsOf(BridgeRangeSelector s, int k) {
  const advanced = true;
  final mode = SelectorSettingItem(AnimatorSetting.mode, advanced);
  final basis = SelectorSettingItem(AnimatorSetting.basis, advanced);
  final shape = SelectorSettingItem(AnimatorSetting.shape, advanced);
  if (s.mode != BridgeSelectorMode.range) {
    // The spoken words pick the letters: what is counted and how a word
    // changes.
    return [
      const SelectorSettingItem(AnimatorSetting.basis),
      const SelectorSettingItem(AnimatorSetting.shape),
      if (k > 0) const SelectorSettingItem(AnimatorSetting.mode),
    ];
  }
  return switch (s.kind) {
    BridgeSelectorKind.range => [
        const SelectorSettingItem(AnimatorSetting.selector),
        const SelectorValueItem(TextAnimatorValue.rangeStart),
        const SelectorValueItem(TextAnimatorValue.rangeEnd),
        const SelectorValueItem(TextAnimatorValue.rangeOffset),
        const SelectorSettingItem(AnimatorSetting.units, advanced),
        basis,
        mode,
        shape,
        const SelectorValueItem(TextAnimatorValue.easeHigh, advanced),
        const SelectorValueItem(TextAnimatorValue.easeLow, advanced),
        const SelectorSettingItem(AnimatorSetting.randomize, advanced),
        if (s.randomize) const SelectorSettingItem(AnimatorSetting.seed, advanced),
      ],
    BridgeSelectorKind.stagger => [
        const SelectorSettingItem(AnimatorSetting.selector),
        const SelectorValueItem(TextAnimatorValue.staggerAmount),
        const SelectorValueItem(TextAnimatorValue.staggerDelay),
        const SelectorSettingItem(AnimatorSetting.delayUnit),
        const SelectorSettingItem(AnimatorSetting.total),
        const SelectorValueItem(TextAnimatorValue.staggerDelayEase),
        const SelectorSettingItem(AnimatorSetting.order),
        if (s.order == BridgeStaggerOrder.random)
          const SelectorSettingItem(AnimatorSetting.seed),
        const SelectorSettingItem(AnimatorSetting.repeat),
        const SelectorSettingItem(AnimatorSetting.bouncePreset),
        const SelectorValueItem(TextAnimatorValue.staggerBounceAmplitude),
        const SelectorValueItem(TextAnimatorValue.staggerBounceFrequency),
        const SelectorValueItem(TextAnimatorValue.staggerBounceDecay),
        const SelectorSettingItem(AnimatorSetting.bounceLast),
        basis,
        mode,
      ],
    BridgeSelectorKind.wiggly => [
        const SelectorSettingItem(AnimatorSetting.selector),
        const SelectorValueItem(TextAnimatorValue.maxAmount),
        const SelectorValueItem(TextAnimatorValue.minAmount),
        const SelectorValueItem(TextAnimatorValue.wiggles),
        const SelectorValueItem(TextAnimatorValue.correlation),
        const SelectorSettingItem(AnimatorSetting.seed),
        basis,
        if (k > 0) mode,
      ],
    BridgeSelectorKind.proximity => [
        const SelectorSettingItem(AnimatorSetting.selector),
        const SelectorSettingItem(AnimatorSetting.target),
        const SelectorValueItem(TextAnimatorValue.radius),
        if (k == 0) const SelectorSettingItem(AnimatorSetting.invert),
        basis,
        shape,
        if (k > 0) mode,
      ],
  };
}

/// Whether [item] does nothing as [s] stands, so its row is drawn faint: the
/// bounce's frequency, decay and Bounce after while there is no bounce, the
/// eases on a Square shape.
bool selectorItemIdle(BridgeRangeSelector s, SelectorItem item) {
  final noBounce = s.bounceAmplitude == const BridgeScalar.static_(0);
  return switch (item) {
    SelectorValueItem(
      value: TextAnimatorValue.staggerBounceFrequency ||
          TextAnimatorValue.staggerBounceDecay
    ) =>
      noBounce,
    SelectorSettingItem(setting: AnimatorSetting.bounceLast) => noBounce,
    SelectorValueItem(
      value: TextAnimatorValue.easeHigh || TextAnimatorValue.easeLow
    ) =>
      s.shape == BridgeSelectorShape.square,
    _ => false,
  };
}

/// The choice control of [setting] on selector [s] (number [k]), writing
/// through [put]. [fps] turns a Delay shown in frames.
Widget animatorSettingControl(
  BuildContext context, {
  required LayerReference layer,
  required BridgeRangeSelector s,
  required int k,
  required AnimatorSetting setting,
  required ValueChanged<BridgeRangeSelector> put,
  required String keyName,
  bool dense = false,
}) {
  final key = ValueKey<String>(keyName);
  Widget check(bool value, ValueChanged<bool> onChanged) => Align(
        alignment: Alignment.centerLeft,
        child: HouseCheckbox(key: key, value: value, onChanged: onChanged),
      );
  Widget seed() => DragValueField(
        key: key,
        value: s.seed,
        min: 0,
        max: 99999,
        resetTo: 0,
        decimals: 0,
        onChanged: (v) =>
            put(selectorWith(s, seed: v.round().clamp(0, 99999).toInt())),
      );
  return switch (setting) {
    AnimatorSetting.selector => BareDropdown<BridgeSelectorKind>(
        key: key,
        dense: dense,
        value: s.kind,
        options: BridgeSelectorKind.values,
        label: selectorKindLabel,
        onChanged: (v) => put(selectorWithKind(s, v)),
      ),
    AnimatorSetting.target => LayerPicker(
        key: key,
        dense: dense,
        layers: Provider.of<LumitUiState>(context, listen: false).model.layers,
        value: s.target,
        empty: l10n.none,
        missing: l10n.missingLayer,
        offered: (l) => l.layer.internallayerId != layer.internallayerId,
        onChanged: (picked) => put(picked == null
            ? selectorWith(s, clearTarget: true)
            : selectorWith(s, target: picked)),
      ),
    AnimatorSetting.invert =>
      check(s.invert, (on) => put(selectorWith(s, invert: on))),
    AnimatorSetting.mode => BareDropdown<SelectorModeChoice>(
        key: key,
        dense: dense,
        value: selectorModeOf(s),
        options: selectorModesFor(k),
        label: selectorModeLabel,
        onChanged: (m) => put(selectorWithMode(s, m)),
      ),
    AnimatorSetting.units => BareDropdown<BridgeSelectorUnits>(
        key: key,
        dense: dense,
        value: s.units,
        options: BridgeSelectorUnits.values,
        label: (u) => switch (u) {
          BridgeSelectorUnits.percentage => l10n.textAnimatorUnitsPercentage,
          BridgeSelectorUnits.indexed => l10n.textAnimatorUnitsIndex,
        },
        onChanged: (u) => put(selectorWith(s, units: u)),
      ),
    AnimatorSetting.basis => BareDropdown<BridgeSelectorBasis>(
        key: key,
        dense: dense,
        value: s.basis,
        options: BridgeSelectorBasis.values,
        label: selectorBasisLabel,
        onChanged: (b) => put(selectorWith(s, basis: b)),
      ),
    AnimatorSetting.shape => Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Expanded(
            child: BareDropdown<BridgeSelectorShape>(
              key: key,
              dense: dense,
              value: s.shape,
              options: BridgeSelectorShape.values,
              label: selectorShapeLabel,
              onChanged: (v) => put(selectorWith(s, shape: v)),
            ),
          ),
          const SizedBox(width: 4),
          SelectorCurvePreview(selector: s),
        ],
      ),
    AnimatorSetting.randomize =>
      check(s.randomize, (on) => put(selectorWith(s, randomize: on))),
    AnimatorSetting.seed => dense ? SizedBox(width: 72, child: seed()) : seed(),
    AnimatorSetting.order => BareDropdown<BridgeStaggerOrder>(
        key: key,
        dense: dense,
        value: s.order,
        options: BridgeStaggerOrder.values,
        label: staggerOrderLabel,
        onChanged: (o) => put(selectorWith(s, order: o)),
      ),
    AnimatorSetting.delayUnit => BareDropdown<bool>(
        key: key,
        dense: dense,
        value: s.delayFrames,
        options: const [false, true],
        label: (frames) => frames
            ? l10n.textAnimatorDelayFrames
            : l10n.textAnimatorDelaySeconds,
        onChanged: (frames) => put(selectorWith(s, delayFrames: frames)),
      ),
    AnimatorSetting.total =>
      check(s.total, (on) => put(selectorWith(s, total: on))),
    AnimatorSetting.repeat => BareDropdown<BridgeStaggerRepeat>(
        key: key,
        dense: dense,
        value: s.repeat,
        options: BridgeStaggerRepeat.values,
        label: (r) => switch (r) {
          BridgeStaggerRepeat.once => l10n.textAnimatorRepeatOnce,
          BridgeStaggerRepeat.loop => l10n.textAnimatorRepeatLoop,
          BridgeStaggerRepeat.pingPong => l10n.textAnimatorRepeatPingPong,
        },
        onChanged: (r) => put(selectorWith(s, repeat: r)),
      ),
    AnimatorSetting.bouncePreset => BareDropdown<BouncePreset>(
        key: key,
        dense: dense,
        value: bouncePresetOf(s),
        // Custom is what the numbers say, not something to pick.
        options: [
          for (final p in BouncePreset.values)
            if (p != BouncePreset.custom || bouncePresetOf(s) == p) p,
        ],
        label: (p) => p.label,
        onChanged: (p) => put(selectorWithBounce(s, p)),
      ),
    AnimatorSetting.bounceLast => BareDropdown<bool>(
        key: key,
        dense: dense,
        value: s.bounceLast,
        options: const [false, true],
        label: (last) => last
            ? l10n.textAnimatorBounceLastKey
            : l10n.textAnimatorBounceEveryKey,
        onChanged: (last) => put(selectorWith(s, bounceLast: last)),
      ),
  };
}

/// Whether [setting] on [s] differs from a fresh selector, for the
/// Timeline's Modified filter.
bool animatorSettingChanged(BridgeRangeSelector s, AnimatorSetting setting) =>
    switch (setting) {
      AnimatorSetting.selector => s.kind != BridgeSelectorKind.range,
      AnimatorSetting.target => s.target != null,
      AnimatorSetting.invert => s.invert,
      AnimatorSetting.mode =>
        selectorModeOf(s) != SelectorModeChoice.add,
      AnimatorSetting.units => s.units != BridgeSelectorUnits.percentage,
      AnimatorSetting.basis => s.basis != BridgeSelectorBasis.characters,
      AnimatorSetting.shape => s.shape != BridgeSelectorShape.square,
      AnimatorSetting.randomize => s.randomize,
      AnimatorSetting.seed => s.seed != 0,
      AnimatorSetting.order => s.order != BridgeStaggerOrder.forward,
      AnimatorSetting.delayUnit => s.delayFrames,
      AnimatorSetting.total => s.total,
      AnimatorSetting.repeat => s.repeat != BridgeStaggerRepeat.once,
      AnimatorSetting.bouncePreset =>
        bouncePresetOf(s) != BouncePreset.none,
      AnimatorSetting.bounceLast => s.bounceLast,
    };

/// How a Delay shown in frames is scaled: the comp's frames a second, or 1
/// for seconds.
double delayScale(BridgeRangeSelector s, CompositionReference comp) =>
    s.delayFrames ? comp.fps() : 1.0;

/// The Advanced twirls standing open, by row key, shared by both panels'
/// selectors so a twirl stays open across a redraw.
final ValueNotifier<Set<String>> advancedOpen = ValueNotifier(<String>{});

void toggleAdvanced(String key) {
  final next = {...advancedOpen.value};
  if (!next.remove(key)) next.add(key);
  advancedOpen.value = next;
}

// ---- The curve preview ----------------------------------------------------

/// The per-letter curve a selector draws, small, beside its Shape (or a
/// Stagger's Amount): weight up, the letters (or, for a Stagger, Amount's
/// keys) along. Drawn at the playhead for a Range, whose window moves.
class SelectorCurvePreview extends StatelessWidget {
  final BridgeRangeSelector selector;
  const SelectorCurvePreview({super.key, required this.selector});

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final t = ThemeScope.of(context).theme;
    return ValueListenableBuilder<int>(
      valueListenable: ui.playheadFrame,
      builder: (context, frame, _) {
        final comp = ui.selectedComp;
        final seconds = comp == null
            ? 0.0
            : rationalSeconds(comp.timeOfFrame(frame: frame));
        return CustomPaint(
          size: const Size(36, 16),
          painter: _CurvePainter(
            weights: selectorPreviewWeights(selector, seconds),
            ink: t.textSecondary,
            ground: t.hairline,
          ),
        );
      },
    );
  }
}

/// 32 samples of [s]'s curve at [seconds]: for a Range the weight across
/// the words (a run of ten units when it counts in Index), for a Stagger its
/// Amount across its keys, for the rest the plain shape.
List<double> selectorPreviewWeights(BridgeRangeSelector s, double seconds) {
  const n = 32;
  double at(BridgeScalar v) => evaluateScalar(v, seconds);
  double shaped(double u) {
    u = u.clamp(0.0, 1.0);
    return switch (s.shape) {
      BridgeSelectorShape.square => u >= 1 ? 1 : 0,
      BridgeSelectorShape.ramp => u,
      BridgeSelectorShape.smooth => u * u * (3 - 2 * u),
      BridgeSelectorShape.bounce => () {
          const k = 1.70158;
          final v = u - 1;
          return 1 + (k + 1) * v * v * v + k * v * v;
        }(),
    };
  }

  if (s.kind == BridgeSelectorKind.stagger) {
    final keys = keysOf(s.amount);
    if (keys.length < 2) {
      return List.filled(n, at(s.amount) / 100);
    }
    final t0 = rationalSeconds(keys.first.time);
    final t1 = rationalSeconds(keys.last.time);
    return [
      for (var i = 0; i < n; i++)
        evaluateScalar(s.amount, t0 + (t1 - t0) * i / (n - 1)) / 100,
    ];
  }
  if (s.kind != BridgeSelectorKind.range) {
    return [for (var i = 0; i < n; i++) shaped(1 - i / (n - 1))];
  }
  final whole = s.units == BridgeSelectorUnits.indexed ? 10.0 : 100.0;
  final offset = at(s.offset);
  var lo = (at(s.start) + offset) / whole;
  var hi = (at(s.end) + offset) / whole;
  if (lo > hi) (lo, hi) = (hi, lo);
  final high = (at(s.easeHigh) / 100).clamp(-1.0, 1.0);
  final low = (at(s.easeLow) / 100).clamp(-1.0, 1.0);
  return [
    for (var i = 0; i < n; i++)
      () {
        final p = (i + 0.5) / n;
        final w = switch (s.shape) {
          BridgeSelectorShape.square => p >= lo && p < hi ? 1.0 : 0.0,
          _ when hi <= lo => p >= hi ? 1.0 : 0.0,
          _ => shaped(_bend((p - lo) / (hi - lo), high, low)),
        };
        return s.invert ? 1 - w : w;
      }(),
  ];
}

/// The eases' bend of `t`, as the engine's `Ease::bend`.
double _bend(double t, double high, double low) {
  if (high == 0 && low == 0) return t.clamp(0.0, 1.0);
  t = t.clamp(0.0, 1.0);
  final p1 = low >= 0 ? (low, 0.0) : (0.0, -low);
  final p2 = high >= 0 ? (1 - high, 1.0) : (1.0, 1 + high);
  double bez(double s, double a, double b) {
    final r = 1 - s;
    return 3 * r * r * s * a + 3 * r * s * s * b + s * s * s;
  }

  var (lo, hi) = (0.0, 1.0);
  for (var i = 0; i < 30; i++) {
    final mid = (lo + hi) / 2;
    if (bez(mid, p1.$1, p2.$1) < t) {
      lo = mid;
    } else {
      hi = mid;
    }
  }
  return bez((lo + hi) / 2, p1.$2, p2.$2).clamp(0.0, 1.0);
}

class _CurvePainter extends CustomPainter {
  final List<double> weights;
  final Color ink, ground;
  _CurvePainter(
      {required this.weights, required this.ink, required this.ground});

  @override
  void paint(Canvas canvas, Size size) {
    final frame = Paint()
      ..color = ground
      ..style = PaintingStyle.stroke
      ..strokeWidth = 1;
    canvas.drawRect(Offset.zero & size, frame);
    if (weights.isEmpty) return;
    // Room for a curve that overshoots by a fifth either way.
    final lo = math.min(0.0, weights.reduce(math.min));
    final hi = math.max(1.0, weights.reduce(math.max));
    final path = Path();
    for (var i = 0; i < weights.length; i++) {
      final x = 1 + (size.width - 2) * i / math.max(1, weights.length - 1);
      final y =
          size.height - 1 - (size.height - 2) * (weights[i] - lo) / (hi - lo);
      i == 0 ? path.moveTo(x, y) : path.lineTo(x, y);
    }
    canvas.drawPath(
        path,
        Paint()
          ..color = ink
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.2);
  }

  @override
  bool shouldRepaint(_CurvePainter old) =>
      old.ink != ink || !_same(old.weights, weights);

  static bool _same(List<double> a, List<double> b) {
    if (a.length != b.length) return false;
    for (var i = 0; i < a.length; i++) {
      if (a[i] != b[i]) return false;
    }
    return true;
  }
}

// ---- The staggered copies in the graph editor -----------------------------

/// The seconds each unit of a Stagger [s] starts after the first, distinct
/// ones only, for a text of [units] units: the faint copies the graph
/// editor draws of Amount's curve.
List<double> staggerOffsets(BridgeRangeSelector s, int units, double seconds) {
  if (units <= 1) return const [0];
  final mid = (units - 1) / 2;
  int fromMid(int i) => ((i - mid).abs() + 0.25).floor();
  final half = (mid + 0.25).floor();
  final ranks = <int>{
    for (var i = 0; i < units; i++)
      switch (s.order) {
        BridgeStaggerOrder.fromCentre => fromMid(i),
        BridgeStaggerOrder.fromEdges => half - math.min(fromMid(i), half),
        _ => i,
      },
  };
  final last = ranks.reduce(math.max).toDouble();
  var delay = evaluateScalar(s.delay, seconds);
  if (s.total) delay = last > 0 ? delay / last : 0;
  final ease = (evaluateScalar(s.delayEase, seconds) / 100).clamp(-1.0, 1.0);
  return [
    for (final r in ranks)
      () {
        var p = last > 0 ? r / last : 0.0;
        p = ease >= 0
            ? math.pow(p, 1 + 3 * ease).toDouble()
            : 1 - math.pow(1 - p, 1 - 3 * ease).toDouble();
        return p * last * delay;
      }(),
  ];
}

/// How many units [text] has for [basis], as the engine counts them.
int unitCount(String text, BridgeSelectorBasis basis) {
  final chars = text.runes.toList();
  bool space(int c) => String.fromCharCode(c).trim().isEmpty;
  return switch (basis) {
    BridgeSelectorBasis.characters => chars.length,
    BridgeSelectorBasis.charactersExcludingSpaces =>
      chars.where((c) => !space(c)).length,
    BridgeSelectorBasis.lines =>
      chars.isEmpty ? 0 : 1 + chars.where((c) => c == 10).length,
    BridgeSelectorBasis.words => text
        .split(RegExp(r'\s+'))
        .where((w) => w.isNotEmpty)
        .length,
  };
}

// ---- Copy, paste, duplicate, out --------------------------------------------

/// The animators Copy kept, for Paste on any Text layer.
List<BridgeTextAnimator> copiedAnimators = const [];

/// A name not yet used among [animators]: [name], or it with a number.
String freeAnimatorName(List<BridgeTextAnimator> animators, String name) {
  final taken = {for (final a in animators) a.name};
  if (!taken.contains(name)) return name;
  for (var n = 2;; n++) {
    final next = '$name $n';
    if (!taken.contains(next)) return next;
  }
}

/// [a] with every key moved by [by] seconds' worth of [comp]'s frames.
BridgeScalar shiftedScalar(BridgeScalar v, BridgeRational Function(BridgeRational) move) =>
    switch (v) {
      BridgeScalar_Keyframed(:final field0) => BridgeScalar.keyframed([
          for (final k in field0)
            BridgeKeyframe(
                time: move(k.time),
                value: k.value,
                interpIn: k.interpIn,
                interpOut: k.interpOut),
        ]),
      _ => v,
    };

/// [a]'s every number passed through [f], its selectors' too.
BridgeTextAnimator animatorMapped(
    BridgeTextAnimator a, BridgeScalar Function(BridgeScalar) f) {
  BridgeRangeSelector sel(BridgeRangeSelector s) => selectorWith(s,
      start: f(s.start),
      end: f(s.end),
      offset: f(s.offset),
      radius: f(s.radius),
      easeHigh: f(s.easeHigh),
      easeLow: f(s.easeLow),
      amount: f(s.amount),
      delay: f(s.delay),
      delayEase: f(s.delayEase),
      bounceAmplitude: f(s.bounceAmplitude),
      bounceFrequency: f(s.bounceFrequency),
      bounceDecay: f(s.bounceDecay),
      minAmount: f(s.minAmount),
      maxAmount: f(s.maxAmount),
      wiggles: f(s.wiggles),
      correlation: f(s.correlation));
  var out = animatorWith(a,
      selector: sel(a.selector),
      selectors: [for (final s in a.selectors) sel(s)],
      axes: [
        for (final x in a.axes) BridgeAnimatorAxis(tag: x.tag, value: f(x.value)),
      ]);
  for (final v in TextAnimatorValue.values) {
    if (animatorPropertyOf(v) != null || v == TextAnimatorValue.animatorAmount) {
      out = animatorWithScalar(out, v, f(textAnimatorScalarOf(out, v)));
    }
  }
  return out;
}

/// The first key time among all of [animators]' numbers, seconds; null
/// when none is keyed.
double? firstKeySeconds(List<BridgeTextAnimator> animators) {
  double? first;
  for (final a in animators) {
    animatorMapped(a, (v) {
      for (final k in keysOf(v)) {
        final s = rationalSeconds(k.time);
        if (first == null || s < first!) first = s;
      }
      return v;
    });
  }
  return first;
}

/// [a] played backwards and moved so its first key is at [frame]: the out
/// animation of an in. Named "[name] out".
BridgeTextAnimator outAnimator(
    BridgeTextAnimator a, CompositionReference comp, int frame,
    List<BridgeTextAnimator> others) {
  final reversed = animatorMapped(a, reversedScalar);
  final first = firstKeySeconds([reversed]);
  final at = rationalSeconds(comp.timeOfFrame(frame: frame));
  final by = first == null ? 0.0 : at - first;
  final fps = comp.fps();
  // Whole frames, so the keys stay on the comp's frames.
  final frames = (by * fps).round();
  BridgeRational move(BridgeRational t) {
    final f = (rationalSeconds(t) * fps).round() + frames;
    return comp.timeOfFrame(frame: f);
  }

  return animatorWith(animatorMapped(reversed, (v) => shiftedScalar(v, move)),
      name: freeAnimatorName(others, l10n.textAnimatorOutName(a.name)));
}

// ---- User presets -----------------------------------------------------------

/// Where the user's text animator presets are kept: a folder of `.json`
/// files beside the effect presets' folder, not inside it, so the Effects &
/// Presets panel never lists it as a folder of effect presets. One kept inside
/// it by an earlier build is moved out the first time it is looked for.
Directory? animatorPresetsDir() {
  final library = presetsDirPath();
  if (library == null) return null;
  final sep = Platform.pathSeparator;
  final dir = Directory('${Directory(library).parent.path}${sep}text-animator-presets');
  final old = Directory('$library${sep}text-animators');
  try {
    if (old.existsSync() && !dir.existsSync()) old.renameSync(dir.path);
  } catch (_) {
    // Left where it is: the presets are still read from there below.
  }
  return old.existsSync() && !dir.existsSync() ? old : dir;
}

/// The saved presets' names and files, by name.
List<(String, File)> animatorPresets() {
  final dir = animatorPresetsDir();
  if (dir == null || !dir.existsSync()) return const [];
  final files = dir
      .listSync()
      .whereType<File>()
      .where((f) => f.path.toLowerCase().endsWith('.json'))
      .toList()
    ..sort((a, b) => a.path.toLowerCase().compareTo(b.path.toLowerCase()));
  return [
    for (final f in files)
      (
        f.uri.pathSegments.last.replaceAll(RegExp(r'\.json$'), ''),
        f,
      ),
  ];
}

/// Save [animators] as a preset, asking its name. Keys are kept relative to
/// the first one, so the preset lands at the playhead when it is put.
Future<bool> saveAnimatorPreset(
    BuildContext context, List<BridgeTextAnimator> animators) async {
  final dir = animatorPresetsDir();
  if (dir == null || animators.isEmpty) return false;
  final name = await askThemeName(context,
      title: l10n.textAnimatorPresetName, suggested: animators.first.name);
  if (name == null || name.trim().isEmpty) return false;
  final safe = name.trim().replaceAll(RegExp(r'[\\/:*?"<>|]'), '_');
  final first = firstKeySeconds(animators) ?? 0;
  // A thousandth of a second is finer than any frame.
  final from = BridgeRational(num: (first * 1000).round(), den: 1000);
  try {
    final json = await textAnimatorsToJson(animators: animators, from: from);
    dir.createSync(recursive: true);
    File('${dir.path}${Platform.pathSeparator}$safe.json')
        .writeAsStringSync(json);
    return true;
  } catch (_) {
    return false;
  }
}

/// A saved preset's animators, their first key at [frame] of [comp].
Future<List<BridgeTextAnimator>> loadAnimatorPreset(
    File file, CompositionReference comp, int frame) async {
  try {
    final text = file.readAsStringSync();
    // Checked as a list before the engine reads it.
    if (jsonDecode(text) is! List) return const [];
    return await textAnimatorsFromJson(
        json: text, at: comp.timeOfFrame(frame: frame));
  } catch (_) {
    return const [];
  }
}

// ---- An animator's heading ---------------------------------------------------

/// Which animator's name is being edited, as `<layer id>/<index>`, shared so
/// the Timeline and the Effect controls panel agree.
final ValueNotifier<String?> animatorRenaming = ValueNotifier(null);

/// [animators] with animator [index] called [name]; every animator linked
/// to it by its old name follows the new one.
List<BridgeTextAnimator> animatorsRenamed(
    List<BridgeTextAnimator> animators, int index, String name) {
  final was = animators[index].name;
  final typed = name.trim();
  if (typed.isEmpty || typed == was) return animators;
  // Never the name of another animator: Link to and Reverse go by name.
  final next = freeAnimatorName(
      [for (var i = 0; i < animators.length; i++) if (i != index) animators[i]],
      typed);
  return [
    for (var i = 0; i < animators.length; i++)
      if (i == index)
        animatorWith(animators[i], name: next)
      else if (animators[i].follow == was)
        animatorWith(animators[i], follow: next)
      else
        animators[i],
  ];
}

/// [animators] with [from] moved to stand at [to].
List<BridgeTextAnimator> animatorsMoved(
    List<BridgeTextAnimator> animators, int from, int to) {
  if (from == to || from < 0 || from >= animators.length) return animators;
  final out = [...animators];
  final moving = out.removeAt(from);
  out.insert(to.clamp(0, out.length), moving);
  return out;
}

/// The rows of an animator's More menu: duplicate, copy, paste, make out,
/// presets, move, remove.
List<Widget> animatorMoreMenuRows({
  required BuildContext context,
  required String layerId,
  String renameScope = 'fx',
  required VoidCallback close,
  required CompositionReference comp,
  required List<BridgeTextAnimator> animators,
  required int index,
  required ValueChanged<List<BridgeTextAnimator>> write,
}) {
  final a = animators[index];
  void run(void Function() f) {
    close();
    f();
  }

  final frame =
      Provider.of<LumitUiState>(context, listen: false).playheadFrame.value;
  MenuRow row(String key, String label, VoidCallback? onPressed) => MenuRow(
        key: ValueKey<String>('text-animator-$key-$index'),
        enabled: onPressed != null,
        onPressed: onPressed == null ? () {} : () => run(onPressed),
        child: Text(label),
      );
  return [
    row('rename', l10n.rename,
        () => animatorRenaming.value = '$renameScope:$layerId/$index'),
    const MenuDivider(),
    row('duplicate', l10n.textAnimatorDuplicate, () {
      write([
        ...animators.sublist(0, index + 1),
        animatorWith(a, name: freeAnimatorName(animators, a.name), solo: false),
        ...animators.sublist(index + 1),
      ]);
    }),
    row('copy', l10n.textAnimatorCopy, () => copiedAnimators = [a]),
    row('copy-all', l10n.textAnimatorCopyAll,
        () => copiedAnimators = [...animators]),
    row(
        'paste',
        l10n.textAnimatorPaste,
        copiedAnimators.isEmpty
            ? null
            : () {
                write(animatorsPasted(animators, copiedAnimators));
              }),
    const MenuDivider(),
    row(
        'make-out',
        l10n.textAnimatorMakeOut,
        animatorHasKeys(a)
            ? () => write([
                  ...animators,
                  outAnimator(a, comp, frame, animators),
                ])
            : null),
    row('save-preset', l10n.textAnimatorSavePreset,
        () => saveAnimatorPreset(context, [a])),
    row('save-all-preset', l10n.textAnimatorSaveAllPreset,
        () => saveAnimatorPreset(context, animators)),
    const MenuDivider(),
    row('move-up', l10n.moveUp,
        index == 0 ? null : () => write(animatorsMoved(animators, index, index - 1))),
    row(
        'move-down',
        l10n.moveDown,
        index + 1 >= animators.length
            ? null
            : () => write(animatorsMoved(animators, index, index + 1))),
    const MenuDivider(),
    row('reverse-menu', l10n.textAnimatorReverse,
        animatorHasKeys(a)
            ? () => write([
                  for (final other in animators)
                    if (other.name == a.name) reversedAnimator(other) else other,
                ])
            : null),
    row('remove-menu', l10n.textAnimatorRemove, () => write([
          for (var i = 0; i < animators.length; i++)
            if (i != index) animators[i],
        ])),
  ];
}

/// An animator's heading in the Effect controls panel: a grip to drag it
/// above or below the others, its eye and solo, its name (double-click to
/// rename, Enter keeps, Esc goes back), Add, More, Reverse and Remove.
class AnimatorHeading extends StatelessWidget {
  final LayerReference layer;
  final CompositionReference comp;
  final List<BridgeTextAnimator> animators;
  final int index;
  final ValueChanged<List<BridgeTextAnimator>> write;
  final bool solid;

  /// Which panel this heading is in, so a rename opens only here.
  final String renameScope;

  const AnimatorHeading({
    super.key,
    required this.layer,
    required this.comp,
    required this.animators,
    required this.index,
    required this.write,
    required this.solid,
    this.renameScope = 'fx',
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final a = animators[index];
    void put(BridgeTextAnimator next) => write([
          for (var i = 0; i < animators.length; i++)
            if (i == index) next else animators[i],
        ]);
    final layerId = layer.internallayerId.toString();
    // The panel's own: a rename opens where it was asked for.
    final renameKey = '$renameScope:${layer.internallayerId}/$index';
    final name = ValueListenableBuilder<String?>(
      valueListenable: animatorRenaming,
      builder: (context, renaming, _) => renaming == renameKey
          ? FxRenameField(
              initial: a.name,
              onDone: (next) {
                animatorRenaming.value = null;
                write(animatorsRenamed(animators, index, next));
              },
              onCancel: () => animatorRenaming.value = null,
            )
          : LumitTooltip(
              // The whole name, which a narrow column cuts short.
              message: '${a.name}\n${l10n.textAnimatorRename}',
              child: DoubleClickListener(
                key: ValueKey<String>('text-animator-name-$index'),
                onDoubleClick: () => animatorRenaming.value = renameKey,
                child: Text(t.kickerCase(a.name),
                    style: a.enabled ? t.kickerOn : t.kicker,
                    overflow: TextOverflow.ellipsis),
              ),
            ),
    );
    final heading = fxTwoColumnRow(
      context: context,
      name: Row(
        children: [
          // The twirl folds the whole animator to its heading.
          GestureDetector(
            key: ValueKey<String>('text-animator-fold-$index'),
            behavior: HitTestBehavior.opaque,
            onTap: () => toggleAnimatorFold(animatorFoldKey(layer, index)),
            child: SizedBox(
              width: iconSize + 2,
              child: glyph.LumitIcon(
                  animatorFolded.value.contains(animatorFoldKey(layer, index))
                      ? LumitIcons.expand
                      : LumitIcons.collapse,
                  size: iconSize,
                  colour: t.textSecondary),
            ),
          ),
          animatorSwitch(context,
              key: 'text-animator-eye-$index',
              solo: false,
              on: a.enabled,
              onTap: () => put(animatorWith(a, enabled: !a.enabled))),
          animatorSwitch(context,
              key: 'text-animator-solo-$index',
              solo: true,
              on: a.solo,
              onTap: () => put(animatorWith(a, solo: !a.solo))),
          const SizedBox(width: 4),
          Expanded(
            child: animatorDraggable(context,
                layerId: layerId,
                index: index,
                name: a.name,
                child: Row(children: [
                  animatorGrip(context, key: 'text-animator-grip-$index'),
                  Flexible(child: name),
                ])),
          ),
        ],
      ),
      // Only the buttons the column has room for: Reverse, then Remove, give
      // way to More (whose menu holds both) rather than running into the
      // key lane or being cut short.
      control: LayoutBuilder(builder: (context, box) {
        double w(String s) => (TextPainter(
              text: TextSpan(text: s, style: t.small),
              textDirection: TextDirection.ltr,
              textScaler: MediaQuery.textScalerOf(context),
            )..layout())
                .width;
        const arrow = 2 + iconSize;
        final base = w(l10n.shapeOpAdd) + arrow + 8 + w(l10n.textAnimatorMore) + arrow;
        final remove = w(l10n.textAnimatorRemove);
        final reverse = w(l10n.textAnimatorReverse);
        final keyed = animatorHasKeys(a);
        final showRemove = base + 8 + remove + 2 <= box.maxWidth;
        final showReverse =
            keyed && showRemove && base + 8 + reverse + 12 + remove + 2 <= box.maxWidth;
        return Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          // Add ▸ Property or Selector, as After Effects' Add beside an
          // animator.
          RowButton(
            key: ValueKey<String>('text-animator-add-to-$index'),
            label: l10n.shapeOpAdd,
            arrow: true,
            onTap: (at) async {
              final style = layer.getText()?.style;
              if (style != null) {
                await fontAxesReady(style.family, style.face);
              }
              if (!context.mounted) return;
              showMenuAt<void>(
                context: context,
                position: at,
                width: 170,
                rows: (close) => animatorAddMenuRows(
                  close: () => close(null),
                  animators: animators,
                  index: index,
                  style: style,
                  solid: solid,
                  put: put,
                ),
              );
            },
          ),
          const SizedBox(width: 8),
          animatorMoreButton(context,
              layerId: layerId,
              renameScope: renameScope,
              key: 'text-animator-more-$index',
              comp: comp,
              animators: animators,
              index: index,
              write: write),
          if (showReverse)
            ...[
              const SizedBox(width: 8),
              RowButton(
                key: ValueKey<String>('text-animator-reverse-$index'),
                label: l10n.textAnimatorReverse,
                // Every animator of this name, as Tracking in lays two.
                onTap: (_) => write(animatorsReversed(animators, index)),
              ),
            ],
          // A word without an arrow leaves more room after it, so the gaps
          // read alike.
          if (showRemove) ...[
            SizedBox(width: animatorButtonGap(afterArrow: !showReverse)),
            RowButton(
              key: ValueKey<String>('text-animator-remove-$index'),
              label: l10n.textAnimatorRemove,
              onTap: (_) => write([
                for (var i = 0; i < animators.length; i++)
                  if (i != index) animators[i],
              ]),
            ),
          ],
        ],
        );
      }),
    );
    // Renaming, the field takes the whole row, buttons and all, so a long
    // name can be read and edited.
    final renameRow = SizedBox(
      height: fxRowHeight(t),
      child: Row(children: [
        const SizedBox(width: fxKeyColumnWidth + 4),
        Expanded(
          child: LayoutBuilder(
            builder: (context, box) => Align(
              alignment: AlignmentDirectional.centerStart,
              child: FxRenameField(
                initial: a.name,
                width: box.maxWidth - 8,
                onDone: (next) {
                  animatorRenaming.value = null;
                  write(animatorsRenamed(animators, index, next));
                },
                onCancel: () => animatorRenaming.value = null,
              ),
            ),
          ),
        ),
      ]),
    );
    final shown = ValueListenableBuilder<String?>(
      valueListenable: animatorRenaming,
      builder: (context, renaming, _) =>
          renaming == renameKey ? renameRow : heading,
    );
    return animatorDropTarget(
      layerId: layerId,
      index: index,
      onMove: (from) => write(animatorsMoved(animators, from, index)),
      // A right-click on the heading opens More, Rename first.
      child: GestureDetector(
        behavior: HitTestBehavior.translucent,
        onSecondaryTapUp: (d) => showAnimatorMoreMenu(context, d.globalPosition,
            layerId: layerId,
            renameScope: renameScope,
            comp: comp,
            animators: animators,
            index: index,
            write: write),
        child: shown,
      ),
    );
  }
}

/// More ▸, beside an animator's name: what can be done with the whole
/// animator. The Timeline heading opens the same menu on a right-click.
Widget animatorMoreButton(BuildContext context,
        {required String layerId,
        String renameScope = 'fx',
        required String key,
        required CompositionReference comp,
        required List<BridgeTextAnimator> animators,
        required int index,
        required ValueChanged<List<BridgeTextAnimator>> write}) =>
    LumitTooltip(
      message: l10n.textAnimatorMoreTip,
      child: RowButton(
        key: ValueKey<String>(key),
        label: l10n.textAnimatorMore,
        arrow: true,
        onTap: (at) => showAnimatorMoreMenu(context, at,
            layerId: layerId,
            renameScope: renameScope,
            comp: comp, animators: animators, index: index, write: write),
      ),
    );

/// The More menu of animator [index], dropped at [at].
void showAnimatorMoreMenu(BuildContext context, Offset at,
        {required String layerId,
        String renameScope = 'fx',
        required CompositionReference comp,
        required List<BridgeTextAnimator> animators,
        required int index,
        required ValueChanged<List<BridgeTextAnimator>> write}) =>
    showMenuAt<void>(
      context: context,
      position: at,
      width: 200,
      rows: (close) => animatorMoreMenuRows(
        context: context,
        layerId: layerId,
        renameScope: renameScope,
        close: () => close(null),
        comp: comp,
        animators: animators,
        index: index,
        write: write,
      ),
    );

/// The room between two of an animator heading's buttons: an arrow leaves
/// some of its own after the word, a bare word does not.
double animatorButtonGap({required bool afterArrow}) => afterArrow ? 8 : 12;

/// [animators] with animator [index] played backwards: every animator of its
/// name, as Tracking in lays two.
List<BridgeTextAnimator> animatorsReversed(
    List<BridgeTextAnimator> animators, int index) {
  final name = animators[index].name;
  return [
    for (final other in animators)
      if (other.name == name) reversedAnimator(other) else other,
  ];
}

/// An animator dragged by its grip: whose, and its place in the list, so a
/// drop moves it only among its own layer's animators.
class AnimatorDrag {
  final String layerId;
  final int index;
  const AnimatorDrag(this.layerId, this.index);
}

/// The grip that drags an animator above or below the others, in the Effect
/// controls panel and the Timeline alike.
Widget animatorGrip(BuildContext context, {required String key}) {
  final t = ThemeScope.of(context).theme;
  return LumitTooltip(
    message: l10n.textAnimatorDrag,
    child: Padding(
      padding: const EdgeInsetsDirectional.only(end: 4),
      child: Text('⠿',
          key: ValueKey<String>(key),
          style: t.body.copyWith(
              fontSize: 17, height: 1, color: t.textSecondary)),
    ),
  );
}

/// What drags an animator: its grip and its name, held with the left button
/// and moved up or down onto another heading. Only a vertical move starts
/// the drag, so a click on the name (to pick, twirl or rename) still lands.
Widget animatorDraggable(BuildContext context,
    {required String layerId,
    required int index,
    required String name,
    required Widget child}) {
  final t = ThemeScope.of(context).theme;
  return Draggable<AnimatorDrag>(
    data: AnimatorDrag(layerId, index),
    affinity: Axis.vertical,
    dragAnchorStrategy: pointerDragAnchorStrategy,
    feedback: Text(name, style: t.kickerOn),
    childWhenDragging: Opacity(opacity: 0.4, child: child),
    child: MouseRegion(cursor: SystemMouseCursors.grab, child: child),
  );
}

/// How far an animator's rows sit in from its heading in the Effect
/// controls panel, and an Advanced row in from those.
const double animatorRowIndent = 18;

/// The room the Timeline keeps for an animator heading's Add, More and
/// Remove, so they and the Animators heading's Add animator start in one
/// column.
const double animatorButtonsWidth = 260;

/// The animators folded shut in the Effect controls panel, by
/// [animatorFoldKey]. Not saved: a fold is how the panel is looked at.
final ValueNotifier<Set<String>> animatorFolded = ValueNotifier(<String>{});

String animatorFoldKey(LayerReference layer, int index) =>
    '${layer.internallayerId}/$index';

void toggleAnimatorFold(String key) {
  final next = {...animatorFolded.value};
  if (!next.remove(key)) next.add(key);
  animatorFolded.value = next;
}

/// An animator's heading, taking a grip dropped on it: the dragged
/// animator moves to this place, a line over the heading showing where.
Widget animatorDropTarget(
        {required String layerId,
        required int index,
        required ValueChanged<int> onMove,
        required Widget child}) =>
    Builder(builder: (context) {
      final t = ThemeScope.of(context).theme;
      return DragTarget<AnimatorDrag>(
        onWillAcceptWithDetails: (d) =>
            d.data.layerId == layerId && d.data.index != index,
        onAcceptWithDetails: (d) => onMove(d.data.index),
        builder: (context, over, _) => over.isEmpty
            ? child
            : DecoratedBox(
                position: DecorationPosition.foreground,
                decoration: BoxDecoration(
                    border:
                        Border(top: BorderSide(color: t.accent, width: 2))),
                child: child,
              ),
      );
    });

/// An animator's eye or solo, drawn as a layer's switches are: the eye
/// closes when off, the solo mark dims until it is on.
Widget animatorSwitch(BuildContext context,
    {required String key,
    required bool solo,
    required bool on,
    required VoidCallback onTap}) {
  final t = ThemeScope.of(context).theme;
  return LumitTooltip(
    message: solo ? l10n.textAnimatorSolo : l10n.textAnimatorEnabled,
    child: GestureDetector(
      key: ValueKey<String>(key),
      behavior: HitTestBehavior.opaque,
      onTap: onTap,
      child: SizedBox(
        width: iconSize + 4,
        child: Center(
          child: glyph.LumitIcon(
              solo
                  ? LumitIcons.solo
                  : on
                      ? LumitIcons.visible
                      : LumitIcons.hidden,
              size: iconSize,
              colour: on
                  ? t.textPrimary
                  : solo
                      ? t.textDisabled
                      : t.textMuted),
        ),
      ),
    ),
  );
}

/// Write [layer]'s animators back as [animators], the rest of the document
/// as it is.
void writeTextAnimatorList(
    LayerReference layer, List<BridgeTextAnimator> animators,
    {LumitUiState? ui}) {
  final document = layer.getText();
  if (document == null) return;
  followAnimators(ui, layer, document.animators, animators);
  layer.setText(
    document: BridgeTextDocument(
      text: document.text,
      expression: document.expression,
      size: document.size,
      fill: document.fill,
      path: document.path,
      pathOffset: document.pathOffset,
      style: document.style,
      paragraph: document.paragraph,
      animators: animators,
    ),
  );
}

/// Calls [onDoubleClick] on a second press within the double-tap time,
/// without taking part in the gesture arena, so a single click on the same
/// row (a twirl, a pick) still lands at once.
class DoubleClickListener extends StatefulWidget {
  final VoidCallback onDoubleClick;
  final Widget child;
  const DoubleClickListener(
      {super.key, required this.onDoubleClick, required this.child});

  @override
  State<DoubleClickListener> createState() => _DoubleClickListenerState();
}

class _DoubleClickListenerState extends State<DoubleClickListener> {
  Duration? _last;

  @override
  Widget build(BuildContext context) => Listener(
        behavior: HitTestBehavior.translucent,
        onPointerDown: (e) {
          final last = _last;
          if (last != null && e.timeStamp - last < kDoubleTapTimeout) {
            _last = null;
            widget.onDoubleClick();
          } else {
            _last = e.timeStamp;
          }
        },
        child: widget.child,
      );
}

/// Where each of [old]'s animators is in [next], or null for one that is
/// gone: by name, the n-th of a name to the n-th of it, and those left over
/// on both sides, when as many are left on each, taken as renamed in order.
/// The engine moves the Node graph's wires by the same rule.
List<int?> animatorMoves(
    List<BridgeTextAnimator> old, List<BridgeTextAnimator> next) {
  int nth(List<BridgeTextAnimator> list, int i) =>
      list.take(i).where((a) => a.name == list[i].name).length;
  final taken = List<bool>.filled(next.length, false);
  final out = <int?>[
    for (var i = 0; i < old.length; i++)
      () {
        final n = nth(old, i);
        for (var j = 0; j < next.length; j++) {
          if (!taken[j] && next[j].name == old[i].name && nth(next, j) == n) {
            taken[j] = true;
            return j;
          }
        }
        return null;
      }(),
  ];
  final lost = [for (var i = 0; i < old.length; i++) if (out[i] == null) i];
  final fresh = [for (var j = 0; j < next.length; j++) if (!taken[j]) j];
  if (lost.length == fresh.length) {
    for (var x = 0; x < lost.length; x++) {
      out[lost[x]] = fresh[x];
    }
  }
  return out;
}

/// [key] with the animator index right after [stem] moved as [moves] says;
/// null when that animator is gone. A key about no animator comes back as it
/// is.
String? movedAnimatorKey(String key, String stem, List<int?> moves) {
  if (!key.startsWith(stem)) return key;
  final rest = key.substring(stem.length);
  final cut = rest.indexOf('/');
  final index = int.tryParse(cut < 0 ? rest : rest.substring(0, cut));
  if (index == null || index >= moves.length) return key;
  final to = moves[index];
  if (to == null) return null;
  return '$stem$to${cut < 0 ? '' : rest.substring(cut)}';
}

/// Keep what the panels show about [layer]'s animators on the animators
/// themselves when the list goes from [old] to [next]: which are folded, which
/// Advanced groups are open, the Timeline's open rows and the picked rows (so
/// the graph editor keeps showing the same curves). A rename in progress is
/// put away.
void followAnimators(LumitUiState? ui, LayerReference layer,
    List<BridgeTextAnimator> old, List<BridgeTextAnimator> next) {
  _knownAnimators[layer.internallayerId.toString()] = next;
  final moves = animatorMoves(old, next);
  // A selector taken out moves the ones after it up.
  final selectorMoves = <int, List<int?>>{
    for (var i = 0; i < old.length; i++)
      if (moves[i] case final to?)
        if (selectorMovesOf(old[i], next[to]) case final sm
            when [for (var k = 0; k < sm.length; k++) sm[k] != k]
                .any((x) => x))
          i: sm,
  };
  var same = old.length == next.length && selectorMoves.isEmpty;
  for (var i = 0; same && i < moves.length; i++) {
    same = moves[i] == i;
  }
  if (same) return;
  final id = layer.internallayerId.toString();
  animatorsMovedNote.value = AnimatorsMoved(id, moves, selectorMoves);
  Set<String> moved(Iterable<String> keys, String stem, {bool timeline = false}) => {
        for (final k in keys)
          if (followedAnimatorKey(k, stem, moves, selectorMoves,
                  timeline: timeline)
              case final m?)
            m,
      };
  animatorFolded.value = moved(animatorFolded.value, '$id/');
  advancedOpen.value = moved(advancedOpen.value, '$id/');
  animatorRenaming.value = null;
  if (ui == null) return;
  final open = ui.folds.timelineOpen;
  final opened = moved(open, '${animatorsPath(id)}/', timeline: true);
  open
    ..clear()
    ..addAll(opened);
  final picked = ui.selectedProperties.value;
  final kept =
      moved(picked, '${animatorsPath(id)}/', timeline: true).toList();
  if (kept.length != picked.length ||
      [for (var i = 0; i < kept.length; i++) kept[i] != picked[i]]
          .any((x) => x)) {
    ui.selectedProperties.value = kept;
  }
}

/// [animators] with [copied] added after them: pasted, or put on from a
/// preset. A name already taken is given a free one, the same for every copy
/// of that name (a preset may lay two of one name on purpose, and Reverse
/// goes by name), and a copy linked to another of the copies is linked to it
/// under its new name, so a pasted pair stays a pair.
List<BridgeTextAnimator> animatorsPasted(
    List<BridgeTextAnimator> animators, List<BridgeTextAnimator> copied) {
  final out = [...animators];
  final taken = {for (final a in animators) a.name};
  final renamed = <String, String>{};
  for (final c in copied) {
    renamed.putIfAbsent(c.name, () {
      var name = c.name;
      for (var n = 2; taken.contains(name); n++) {
        name = '${c.name} $n';
      }
      taken.add(name);
      return name;
    });
  }
  final added = [
    for (final c in copied) animatorWith(c, name: renamed[c.name]),
  ];
  return [
    ...out,
    for (final a in added)
      if (a.follow case final f? when renamed.containsKey(f))
        animatorWith(a, follow: renamed[f])
      else
        a,
  ];
}

/// What [followAnimators] last moved, for a panel holding picks of its own
/// (the Timeline's keys) to move them the same way.
class AnimatorsMoved {
  final String layerId;
  final List<int?> moves;

  /// For each animator that lost a selector, by its old place: where each of
  /// its old selectors went.
  final Map<int, List<int?>> selectorMoves;
  const AnimatorsMoved(this.layerId, this.moves, this.selectorMoves);

  /// [key], a Timeline path (or a key or curve id built on one), moved along.
  String? timelineKey(String key) => followedAnimatorKey(
      key, '${animatorsPath(layerId)}/', moves, selectorMoves,
      timeline: true);
}

final ValueNotifier<AnimatorsMoved?> animatorsMovedNote = ValueNotifier(null);

/// Where each of [old]'s selectors is in [next], the same animator before
/// and after an edit: one fewer is the first that differs taken out, as the
/// engine reads it for the Node graph's wires.
List<int?> selectorMovesOf(BridgeTextAnimator old, BridgeTextAnimator next) {
  final before = selectorCount(old), after = selectorCount(next);
  if (after + 1 == before) {
    var gone = before - 1;
    for (var k = 0; k < after; k++) {
      if (selectorAt(old, k) != selectorAt(next, k)) {
        gone = k;
        break;
      }
    }
    return [
      for (var k = 0; k < before; k++)
        k < gone
            ? k
            : k == gone
                ? null
                : k - 1,
    ];
  }
  return [for (var k = 0; k < before; k++) k < after ? k : null];
}

/// The names a selector's rows go by under their animator on the Timeline.
final Set<String> _selectorRowNames = {
  'selector',
  'advanced',
  for (final s in AnimatorSetting.values) s.name,
  for (final kind in BridgeSelectorKind.values)
    for (final item in selectorItemsOf(freshSelector(kind), 0))
      if (item case SelectorValueItem(:final value)) value.name,
};

/// [key] with the animator after [stem] moved as [moves] says, and the
/// selector after it as [selectors] says for that animator; null when either
/// is gone. On the Timeline ([timeline]) a selector is named by its `s<k>/`
/// segment, the first one by no segment; in the Effect controls panel\x27s
/// Advanced keys by a plain `/<k>`. A key about no animator comes back as it
/// is.
String? followedAnimatorKey(String key, String stem, List<int?> moves,
    Map<int, List<int?>> selectors,
    {bool timeline = false}) {
  if (!key.startsWith(stem)) return key;
  final rest = key.substring(stem.length);
  var cut = rest.indexOf('/');
  for (final mark in ['#', '@']) {
    final at = rest.indexOf(mark);
    if (at >= 0 && (cut < 0 || at < cut)) cut = at;
  }
  final index = int.tryParse(cut < 0 ? rest : rest.substring(0, cut));
  if (index == null || index >= moves.length) return key;
  final to = moves[index];
  if (to == null) return null;
  var tail = cut < 0 ? '' : rest.substring(cut);
  final sm = selectors[index];
  if (sm != null && tail.startsWith('/')) {
    final body = tail.substring(1);
    int? k;
    var after = body;
    if (timeline) {
      final m = RegExp(r'^s(\d+)/').firstMatch(body);
      if (m != null) {
        k = int.parse(m.group(1)!);
        after = body.substring(m.end);
      } else if (_selectorRowNames
          .contains(body.split(RegExp(r'[/#@]')).first)) {
        k = 0;
      }
    } else {
      k = int.tryParse(body.split('/').first);
      after = '';
    }
    if (k != null) {
      if (k >= sm.length) return key;
      final k2 = sm[k];
      if (k2 == null) return null;
      tail = timeline
          ? '/${k2 == 0 ? '' : 's$k2/'}$after'
          : '/$k2';
    }
  }
  return '$stem$to$tail';
}

/// Each Text layer's animators as the panels last saw them, by layer id: what
/// [watchAnimatorOrder] compares the document against.
final Map<String, List<BridgeTextAnimator>> _knownAnimators = {};
final Expando<bool> _watching = Expando<bool>();

bool _sameOrder(List<BridgeTextAnimator> a, List<BridgeTextAnimator> b) {
  if (a.length != b.length) return false;
  for (var i = 0; i < a.length; i++) {
    if (a[i].name != b[i].name || selectorCount(a[i]) != selectorCount(b[i])) {
      return false;
    }
  }
  return true;
}

/// Keep what the panels show about each animator on the animator through
/// edits that do not pass through the panels: undo and redo above all, which
/// put an older list back. Each time the model changes, a layer whose
/// animators are in another order than last seen has its folds, Advanced
/// groups, open rows, picks and picked keys moved as [followAnimators] moves
/// them for an edit made here. Installed once per [ui].
void watchAnimatorOrder(LumitUiState ui) {
  if (_watching[ui] == true) return;
  _watching[ui] = true;
  var busy = false;
  void look() {
    if (busy) return;
    busy = true;
    try {
      for (final entry in ui.model.layers) {
        final now = entry.info.textAnimators;
        final id = entry.layer.internallayerId.toString();
        final was = _knownAnimators[id];
        if (was != null && !_sameOrder(was, now)) {
          followAnimators(ui, entry.layer, was, now);
        }
        _knownAnimators[id] = now;
      }
    } finally {
      busy = false;
    }
  }

  look();
  // Never in the middle of a build: the folds it moves rebuild panels.
  ui.model.addListener(() {
    if (SchedulerBinding.instance.schedulerPhase ==
        SchedulerPhase.persistentCallbacks) {
      SchedulerBinding.instance.addPostFrameCallback((_) => look());
    } else {
      look();
    }
  });
}
