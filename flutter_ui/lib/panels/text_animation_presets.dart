// Text animations you pick by name: a fresh animator with its rows set and a
// second of keys laid from the playhead.
//
// Most of them slide a soft-edged range off the words. A Ramp selector gives
// every letter past its end the full offset and every letter before its start
// none, so a range a quarter of the words wide, slid from just before the first
// letter to just past the last, lets go of each letter in its turn. Type on is
// the hard edge of that, on Range start.

import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../l10n/strings.dart';
import '../shell/menu_bar_frb.dart' show MenuEntry;
import '../shell/menu_layer_frb.dart' show onSelection;
import 'graph_maths.dart' show easyEase;
import 'text_animator_rows_frb.dart';

/// How far the soft edge of a range reaches, per cent of the words.
const double _softEdge = 25;

/// How far Rise and Fall travel, and how wide Tracking in starts, in pixels.
const double _travel = 60;
const double _spread = 400;

enum TextAnimationPreset {
  fadeByLetter,
  fadeByWord,
  typeOn,
  riseByLetter,
  fallByWord,
  scaleByLetter,
  trackingIn;

  String get title => switch (this) {
        fadeByLetter => l10n.textPresetFadeByLetter,
        fadeByWord => l10n.textPresetFadeByWord,
        typeOn => l10n.textPresetTypeOn,
        riseByLetter => l10n.textPresetRiseByLetter,
        fallByWord => l10n.textPresetFallByWord,
        scaleByLetter => l10n.textPresetScaleByLetter,
        trackingIn => l10n.textPresetTrackingIn,
      };
}

/// The animators [preset] adds, its keys from [frame] for a second.
List<BridgeTextAnimator> textPresetAnimators(
  TextAnimationPreset preset,
  CompositionReference comp,
  int frame,
) {
  final last = frame + comp.fps().round();
  BridgeScalar sweep(double from, double to, {bool eased = true}) {
    final ease = eased ? easyEase : const BridgeSideInterp.linear();
    BridgeKeyframe key(int at, double value) => BridgeKeyframe(
        time: comp.timeOfFrame(frame: at),
        value: value,
        interpIn: ease,
        interpOut: ease);
    return BridgeScalar.keyframed([key(frame, from), key(last, to)]);
  }

  /// A soft edge sliding off the words: the letters it has left are as they
  /// should be, the ones ahead of it wear the animator's rows in full.
  BridgeRangeSelector reveal(BridgeSelectorBasis basis) => BridgeRangeSelector(
        start: const BridgeScalar.static_(0),
        end: const BridgeScalar.static_(_softEdge),
        offset: sweep(-_softEdge, 100),
        basis: basis,
        shape: BridgeSelectorShape.ramp,
      );

  final fresh = freshTextAnimator(preset.title);
  const hidden = BridgeScalar.static_(0);
  const characters = BridgeSelectorBasis.characters;
  const words = BridgeSelectorBasis.words;
  return switch (preset) {
    TextAnimationPreset.fadeByLetter => [
        animatorWith(fresh, selector: reveal(characters), opacity: hidden)
      ],
    TextAnimationPreset.fadeByWord => [
        animatorWith(fresh, selector: reveal(words), opacity: hidden)
      ],
    TextAnimationPreset.typeOn => [
        animatorWith(
          fresh,
          selector: BridgeRangeSelector(
            start: sweep(0, 100, eased: false),
            end: const BridgeScalar.static_(100),
            offset: const BridgeScalar.static_(0),
            basis: characters,
            shape: BridgeSelectorShape.square,
          ),
          opacity: hidden,
        )
      ],
    TextAnimationPreset.riseByLetter => [
        animatorWith(fresh,
            selector: reveal(characters),
            opacity: hidden,
            positionY: const BridgeScalar.static_(_travel))
      ],
    TextAnimationPreset.fallByWord => [
        animatorWith(fresh,
            selector: reveal(words),
            opacity: hidden,
            positionY: const BridgeScalar.static_(-_travel))
      ],
    TextAnimationPreset.scaleByLetter => [
        animatorWith(fresh,
            selector: reveal(characters),
            opacity: hidden,
            scaleX: const BridgeScalar.static_(0),
            scaleY: const BridgeScalar.static_(0))
      ],
    // Two of them, as an offset only ever pushes one way: one spreads the
    // letters across the words, and one brings the spread back to the middle.
    TextAnimationPreset.trackingIn => [
        animatorWith(
          fresh,
          selector: BridgeRangeSelector(
            start: const BridgeScalar.static_(0),
            end: const BridgeScalar.static_(100),
            offset: const BridgeScalar.static_(0),
            basis: characters,
            shape: BridgeSelectorShape.ramp,
          ),
          positionX: sweep(_spread, 0),
        ),
        animatorWith(fresh,
            positionX: sweep(-_spread / 2, 0), opacity: sweep(0, 100)),
      ],
  };
}

/// One preset on [layer], as one edit of its document. False on a layer with no
/// text.
bool addTextPreset(
  LayerReference layer,
  TextAnimationPreset preset,
  CompositionReference comp,
  int frame,
) =>
    addTextAnimator(layer,
        adding: textPresetAnimators(preset, comp, frame));

/// A blank animator, then the presets, for every menu that offers them.
List<MenuEntry> textPresetRows(LumitState app, LumitUiState ui) {
  bool isText(BridgeLayerEntry entry) => entry.info.kind == BridgeLayerKind.text;
  final comp = ui.selectedComp;
  return [
    MenuEntry(l10n.textAnimatorBlank,
        onSelection(app, ui, (entry) => addTextAnimator(entry.layer),
            when: isText, name: 'Animate text')),
    MenuEntry.divider(),
    for (final preset in TextAnimationPreset.values)
      MenuEntry(
        preset.title,
        comp == null
            ? null
            : onSelection(
                app,
                ui,
                (entry) => addTextPreset(
                    entry.layer, preset, comp, ui.playheadFrame.value),
                when: isText,
                name: preset.title,
              ),
      ),
  ];
}

/// [scalar] played backwards: the same key times, the values the other way
/// round, each side's ease with them. Unkeyed comes back as it was.
BridgeScalar reversedScalar(BridgeScalar scalar) {
  if (scalar is! BridgeScalar_Keyframed) return scalar;
  final keys = scalar.field0;
  final n = keys.length;
  return BridgeScalar.keyframed([
    for (var i = 0; i < n; i++)
      BridgeKeyframe(
        time: keys[i].time,
        value: keys[n - 1 - i].value,
        interpIn: keys[n - 1 - i].interpOut,
        interpOut: keys[n - 1 - i].interpIn,
      ),
  ]);
}

/// [a] played backwards, so an in becomes an out.
BridgeTextAnimator reversedAnimator(BridgeTextAnimator a) {
  final s = a.selector;
  return animatorWith(
    a,
    selector: BridgeRangeSelector(
      start: reversedScalar(s.start),
      end: reversedScalar(s.end),
      offset: reversedScalar(s.offset),
      basis: s.basis,
      shape: s.shape,
    ),
    positionX: reversedScalar(a.positionX),
    positionY: reversedScalar(a.positionY),
    rotation: reversedScalar(a.rotation),
    positionZ: reversedScalar(a.positionZ),
    rotationX: reversedScalar(a.rotationX),
    rotationY: reversedScalar(a.rotationY),
    scaleX: reversedScalar(a.scaleX),
    scaleY: reversedScalar(a.scaleY),
    opacity: reversedScalar(a.opacity),
    fillR: reversedScalar(a.fillR),
    fillG: reversedScalar(a.fillG),
    fillB: reversedScalar(a.fillB),
  );
}
