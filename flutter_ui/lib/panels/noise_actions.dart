// The clean-up effects' two commands, from a key or the Effect menu.
//
// **Learn** presses the Learn button of the effect it finds, and **Removed
// only** turns that effect over to showing, or playing, only what it takes
// away, and back. Which effect: the one picked in Effect controls if it is a
// clean-up effect, otherwise the first clean-up effect on the selected layer,
// top of the stack first.

import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'effect_param_row_frb.dart' show EffectStackEditor;
import 'viewer_noise_sample.dart' show learnNoise;

/// The effects with a Learn button.
const Set<String> _learns = {'denoise', 'audio_denoise'};

/// Each clean-up effect's switch for "only what it takes away": Denoise's
/// view menu, and each sound effect's own box.
const Map<String, String> _onlyRow = {
  'denoise': 'view',
  'audio_denoise': 'noise_only',
  'audio_dehum': 'hum_only',
  'audio_declick': 'clicks_only',
  'audio_deess': 'ess_only',
  'audio_dereverb': 'reverb_only',
  'audio_voice': 'background_only',
};

/// Denoise's view menu: Result, then Removed noise.
const int _result = 0, _removed = 1;

typedef _Found = (LayerReference, BridgeEffectInstanceInfo);

/// The clean-up effect the commands act on, if there is one.
_Found? _target(LumitUiState ui, bool Function(String name) wanted) {
  final owner = ui.selectedEffectsLayer;
  if (owner != null) {
    final entry = ui.model.byId(owner.internallayerId);
    for (final fx in entry?.info.effects ?? const <BridgeEffectInstanceInfo>[]) {
      if (ui.selectedEffects.value.contains(fx.id) && wanted(fx.name)) {
        return (owner, fx);
      }
    }
  }
  for (final layer in ui.selectedLayers.value) {
    final entry = ui.model.byId(layer.internallayerId);
    for (final fx in entry?.info.effects ?? const <BridgeEffectInstanceInfo>[]) {
      if (fx.enabled && wanted(fx.name)) return (layer, fx);
    }
  }
  return null;
}

/// Whether Learn has something to press.
bool canLearnNoise(LumitUiState ui) => _target(ui, _learns.contains) != null;

/// Whether Removed only has something to turn.
bool canShowRemoved(LumitUiState ui) =>
    _target(ui, _onlyRow.containsKey) != null;

/// Whether the effect found shows or plays only what it takes away.
bool removedOnlyOn(LumitUiState ui) {
  final found = _target(ui, _onlyRow.containsKey);
  if (found == null) return false;
  final row = _onlyRow[found.$2.name]!;
  return switch (found.$2.values.where((v) => v.id == row).firstOrNull?.value) {
    BridgeEffectValue_Choice(:final field0) => field0 == _removed,
    BridgeEffectValue_Bool(:final field0) => field0,
    _ => false,
  };
}

/// Press Learn on the effect found. False when there is none.
bool learnSelectedNoise(LumitUiState ui) {
  final found = _target(ui, _learns.contains);
  if (found == null) return false;
  learnNoise(found.$1, found.$2.id, ui);
  return true;
}

/// Turn the effect found over to only what it takes away, or back. False
/// when there is none.
bool toggleRemovedOnly(LumitUiState ui) {
  final found = _target(ui, _onlyRow.containsKey);
  if (found == null) return false;
  final (layer, fx) = found;
  final row = _onlyRow[fx.name]!;
  final held = fx.values.where((v) => v.id == row).firstOrNull?.value;
  final BridgeEffectValue next = switch (held) {
    BridgeEffectValue_Choice(:final field0) =>
      BridgeEffectValue.choice(field0 == _removed ? _result : _removed),
    BridgeEffectValue_Bool(:final field0) => BridgeEffectValue.bool(!field0),
    _ => row == 'view'
        ? const BridgeEffectValue.choice(_removed)
        : const BridgeEffectValue.bool(true),
  };
  EffectStackEditor().writeAll(layer, fx.id, {row: next});
  return true;
}
