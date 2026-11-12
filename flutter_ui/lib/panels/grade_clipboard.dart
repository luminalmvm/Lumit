// A grade as something to carry: Copy grade, Paste grade, and laying a saved
// look on a shot.
//
// In plain terms: a grade is the shot's colour effects, the picked Cut clip's
// own when a clip is picked, else the layer's. These go through the same text
// Copy effect and Paste effect pass around, and a grade put on a shot takes
// the place of the one it had, so picking a second look or pasting over a
// graded shot doesn't stack one on top of the other.

import 'dart:convert';

import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

import '../state/clipboard.dart';
import '../state/grade_target.dart';
import 'layer_fold_frb.dart' show asOneUndoStep;

/// The group of the preset library that looks live in, made on the first save.
const String looksGroup = 'Looks';

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
List<UuidValue> gradeEffectIds(BridgeLayerInfo info) =>
    [for (final e in gradeNodesOf(info.effects)) e.id];

/// The stack a shot's grade lives in, off the read model: the clip's own for
/// a clip. Ids and names are all this is read for, so the clip's clock does
/// not matter here.
List<BridgeEffectInstanceInfo>? shotStackOf(
    LumitUiState ui, GradeShot shot) {
  final info = ui.model.byId(shot.layer.internallayerId)?.info;
  if (info == null || !gradeable(info.kind)) return null;
  final clip = shot.clip;
  if (clip == null) return info.effects;
  for (final c in info.clips) {
    if (c.id == clip) return c.effects;
  }
  return null;
}

/// The grade on [shot], in stack order.
List<UuidValue> shotGradeIds(LumitUiState ui, GradeShot shot) =>
    [for (final e in gradeNodesOf(shotStackOf(ui, shot) ?? const [])) e.id];

/// The shots a grade would land on: the picked Cut clips when the current
/// shot is a clip, else the picked layers, less those that can't take one.
List<GradeShot> gradeTargets(LumitUiState ui) {
  final shot = ui.gradeShot.value;
  if (shot == null) return const [];
  final comp = ui.selectedComp?.internalid;
  final shots = shot.clip == null
      ? [for (final layer in ui.selectedLayers.value) GradeShot(layer)]
      : [
          shot,
          for (final c in ui.cutClips.value)
            if (c.clip != shot.clip && c.layer.internalcompId == comp)
              GradeShot(c.layer, c.clip),
        ];
  return [
    for (final s in shots)
      if (shotStackOf(ui, s) != null) s,
  ];
}

/// Whether the current shot has a grade to copy.
bool canCopyGrade(LumitUiState ui) {
  final shot = ui.gradeShot.value;
  return shot != null && shotGradeIds(ui, shot).isNotEmpty;
}

/// Put the current shot's grade on the effect clipboard. False when it has
/// none.
bool copyGrade(LumitUiState ui) {
  if (!canCopyGrade(ui)) return false;
  final shot = ui.gradeShot.value!;
  try {
    ui.copyEffectsToClipboard(
        shot.layer.copyEffects(effects: shotGradeIds(ui, shot)));
  } catch (_) {
    // The shot has gone; the clipboard keeps what it had.
    return false;
  }
  return true;
}

/// Whether the clipboard holds a grade and something picked can take it. A
/// copied Glow isn't a grade, even with Exposure beside it.
bool canPasteGrade(LumitUiState ui) {
  if (ui.clipboard.kind != ClipboardKind.effects) return false;
  final text = ui.clipboard.text;
  if (text == null || gradeTargets(ui).isEmpty) return false;
  try {
    final effects =
        (jsonDecode(text) as Map<String, dynamic>)['effects'] as List;
    return effects.isNotEmpty &&
        effects.every(
            (e) => gradeEffectNames.contains(e['effect']['match_name']));
  } catch (_) {
    return false;
  }
}

/// The clipboard's grade onto every picked shot that can take one, as one
/// undo step. False when there is nothing to paste or nothing to take it.
bool pasteGrade(LumitState app, LumitUiState ui) {
  if (!canPasteGrade(ui)) return false;
  layGrade(app, ui, ui.clipboard.text!);
  return true;
}

/// [text], a stack of effects, in place of the grade on every picked shot
/// that can take one, or on [to] alone, as one undo step.
void layGrade(LumitState app, LumitUiState ui, String text,
    {String name = 'Paste grade', List<GradeShot>? to}) {
  final frame = ui.playheadFrame.value;
  final targets = [
    for (final s in to ?? gradeTargets(ui))
      if (shotStackOf(ui, s) != null) (s, shotGradeIds(ui, s).toSet()),
  ];
  asOneUndoStep(app.project, () {
    for (final (shot, old) in targets) {
      try {
        _swapGrade(shot, old, text, frame);
      } catch (_) {
        // A locked layer refuses, and the rest still take it.
      }
    }
  }, name: name);
  app.notifyDocumentChanged();
  ui.model.refresh();
}

/// The new grade goes where the old one started, so effects around it keep
/// their place in the stack.
void _swapGrade(GradeShot shot, Set<UuidValue> old, String text, int frame) {
  final layer = shot.layer;
  final clip = shot.clip;
  List<BridgeEffectInstance> stack() => clip == null
      ? layer.getEffects()
      : layer.getClipEffects(clip: clip, onCompClock: true);
  final handles = stack();
  var at = handles.indexWhere((h) => old.contains(h.id()));
  if (at < 0) at = handles.length;
  for (final h in handles) {
    if (old.contains(h.id())) layer.removeEffect(effect: h);
  }
  final kept = {
    for (final h in handles)
      if (!old.contains(h.id())) h.id(),
  };
  clip == null
      ? layer.pasteEffects(text: text, atFrame: frame)
      : layer.loadPreset(text: text, clip: clip);
  for (final h in stack()) {
    if (kept.contains(h.id())) continue;
    layer.reorderEffect(effect: h, newIndex: at);
    at++;
  }
}
