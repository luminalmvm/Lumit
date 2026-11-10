// Which layers of the fronted composition are caption layers, and the few
// things every caption surface needs from them: the one the Captions panel
// shows, where a caption sits in frames, and the way to bring the panel up.

import 'package:uuid/uuid.dart' show UuidValue;
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/captions.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import '../state/dock.dart';

/// What each text layer turned out to be, by id. A layer is made a caption
/// layer when it is made and never stops being one, so the answer is asked
/// for once per layer rather than on every edit.
final Map<UuidValue, bool> _known = {};

/// Whether [entry] is a caption layer. Asks the engine the first time a text
/// layer is seen, so call it from a listener or a gesture, never a build.
bool isCaptionLayer(BridgeLayerEntry entry) {
  if (entry.info.kind != BridgeLayerKind.text) return false;
  final id = entry.layer.internallayerId;
  final known = _known[id];
  if (known != null) return known;
  try {
    return _known[id] = entry.layer.captions() != null;
  } catch (_) {
    return false;
  }
}

/// Note a layer just made with captions, so nobody has to ask.
void noteCaptionLayer(LayerReference layer) =>
    _known[layer.internallayerId] = true;

/// The fronted composition's caption layers, top of the stack first.
List<BridgeLayerEntry> captionLayersIn(LumitUiState ui) => [
      for (final entry in ui.model.layers)
        if (isCaptionLayer(entry)) entry,
    ];

/// The caption layer the Captions panel shows: the selected layer when it is
/// one, then [picked] when it is still there, then the top one.
BridgeLayerEntry? shownCaptionLayer(
    LumitUiState ui, List<BridgeLayerEntry> layers,
    {UuidValue? picked}) {
  if (layers.isEmpty) return null;
  final selected = ui.selectedLayer.value?.internallayerId;
  for (final id in [selected, picked]) {
    if (id == null) continue;
    for (final entry in layers) {
      if (entry.layer.internallayerId == id) return entry;
    }
  }
  return layers.first;
}

/// The frame comp time [t] falls on, as the engine counts it: rounded down.
/// Worked out here because a two-hour layer has thousands of captions and a
/// question to the engine for each would stall the panel.
int captionFrame(BridgeRational t, (int, int) fps) {
  final a = t.num * fps.$1;
  final b = t.den * fps.$2;
  if (b <= 0) return 0;
  var q = a ~/ b;
  if (a < 0 && a % b != 0) q--;
  return q;
}

/// Put the Captions panel up and in front, where it is in the arrangement or
/// beside Text and Paragraph where it is not.
void frontCaptionsPanel(LumitUiState ui) {
  setPanelVisible(ui.split, Panel.captions, true);
  activatePanelTab(ui.split, Panel.captions);
  ui.activePane.value = Panel.captions.pane();
  ui.workspace.touch();
}

/// Move the playhead to the start of the caption before or after it, on the
/// caption layer the panel would show. Says whether there was one.
bool stepToCaption(LumitUiState ui, {required bool forward}) {
  final comp = ui.selectedComp;
  if (comp == null) return false;
  final entry = shownCaptionLayer(ui, captionLayersIn(ui),
      picked: ui.captionSelection.layer);
  BridgeCaptions? captions;
  try {
    captions = entry?.layer.captions();
  } catch (_) {}
  if (captions == null) return false;
  final fps = ui.model.fpsExact;
  final now = ui.playheadFrame.value;
  int? best;
  for (final cue in captions.cues) {
    final start = cue.start;
    if (start == null) continue;
    final frame = captionFrame(start, fps);
    if (forward ? frame <= now : frame >= now) continue;
    if (best == null || (forward ? frame < best : frame > best)) best = frame;
  }
  if (best == null) return false;
  ui.scrubTo(best);
  return true;
}
