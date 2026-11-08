// A grade as something to carry: Copy grade, Paste grade, and laying a saved
// look on a layer.
//
// In plain terms: a grade is the layer's colour effects. These go through the
// same text Copy effect and Paste effect pass around, and a grade put on a layer
// takes the place of the one it had, so picking a second look or pasting over a
// graded shot doesn't stack one on top of the other.

import 'dart:convert';

import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

import '../state/clipboard.dart';
import 'layer_fold_frb.dart' show asOneUndoStep;

/// The group of the preset library that looks live in, made on the first save.
const String looksGroup = 'Looks';

/// The effects a grade can be made of: everything under Colour.
final Set<String> _gradeNames = {
  for (final e in listEffects())
    if (e.category == 'colour') e.name,
};

/// The layers a grade can sit on: the ones that draw a picture.
bool gradeable(BridgeLayerKind kind) => switch (kind) {
      BridgeLayerKind.camera ||
      BridgeLayerKind.light ||
      BridgeLayerKind.nullLayer ||
      BridgeLayerKind.audio ||
      BridgeLayerKind.bus =>
        false,
      _ => true,
    };

/// The grade on a layer, in stack order.
List<UuidValue> gradeEffectIds(BridgeLayerInfo info) => [
      for (final e in info.effects)
        if (_gradeNames.contains(e.name)) e.id,
    ];

/// The layers a grade would land on: the selection, less those that can't
/// take one. A layer the read model has lost is left out.
List<(LayerReference, BridgeLayerInfo)> _gradeTargets(LumitUiState ui) => [
      for (final layer in ui.selectedLayers.value)
        if (ui.model.byId(layer.internallayerId)?.info case final info?)
          if (gradeable(info.kind)) (layer, info),
    ];

/// Whether the primary layer has a grade to copy.
bool canCopyGrade(LumitUiState ui) {
  final layer = ui.selectedLayer.value;
  final info =
      layer == null ? null : ui.model.byId(layer.internallayerId)?.info;
  return info != null &&
      gradeable(info.kind) &&
      gradeEffectIds(info).isNotEmpty;
}

/// Put the primary layer's grade on the effect clipboard. False when it has none.
bool copyGrade(LumitUiState ui) {
  if (!canCopyGrade(ui)) return false;
  final layer = ui.selectedLayer.value!;
  final info = ui.model.byId(layer.internallayerId)!.info;
  try {
    ui.copyEffectsToClipboard(layer.copyEffects(effects: gradeEffectIds(info)));
  } catch (_) {
    // The layer has gone; the clipboard keeps what it had.
    return false;
  }
  return true;
}

/// Whether the clipboard holds a grade and something selected can take it. A
/// copied Glow isn't a grade, even with Exposure beside it.
bool canPasteGrade(LumitUiState ui) {
  if (ui.clipboard.kind != ClipboardKind.effects) return false;
  final text = ui.clipboard.text;
  if (text == null || _gradeTargets(ui).isEmpty) return false;
  try {
    final effects =
        (jsonDecode(text) as Map<String, dynamic>)['effects'] as List;
    return effects.isNotEmpty &&
        effects.every((e) => _gradeNames.contains(e['effect']['match_name']));
  } catch (_) {
    return false;
  }
}

/// The clipboard's grade onto every selected layer that can take one, as one
/// undo step. False when there is nothing to paste or nothing to take it.
bool pasteGrade(LumitState app, LumitUiState ui) {
  if (!canPasteGrade(ui)) return false;
  layGrade(app, ui, ui.clipboard.text!);
  return true;
}

/// [text], a stack of effects, in place of the grade on every selected layer
/// that can take one, as one undo step.
void layGrade(LumitState app, LumitUiState ui, String text) {
  final frame = ui.playheadFrame.value;
  asOneUndoStep(app.project, () {
    for (final (layer, info) in _gradeTargets(ui)) {
      try {
        _swapGrade(layer, info, text, frame);
      } catch (_) {
        // A locked layer refuses, and the rest still take it.
      }
    }
  });
  app.notifyDocumentChanged();
  ui.model.refresh();
}

/// The new grade goes where the old one started, so effects around it keep
/// their place in the stack.
void _swapGrade(
    LayerReference layer, BridgeLayerInfo info, String text, int frame) {
  final old = gradeEffectIds(info).toSet();
  final handles = layer.getEffects();
  var at = handles.indexWhere((h) => old.contains(h.id()));
  if (at < 0) at = handles.length;
  for (final h in handles) {
    if (old.contains(h.id())) layer.removeEffect(effect: h);
  }
  final kept = {
    for (final h in handles)
      if (!old.contains(h.id())) h.id(),
  };
  layer.pasteEffects(text: text, atFrame: frame);
  for (final h in layer.getEffects()) {
    if (kept.contains(h.id())) continue;
    layer.reorderEffect(effect: h, newIndex: at);
    at++;
  }
}
