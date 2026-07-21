// The Timeline lane keyframe selection: a value-identity id for one key, the
// modifier-aware click that grows or replaces the selection (egui's
// `lane_select_click`), and the grouping the drag commit leans on to fire one
// `shiftKeyframes` per (layer, property). Pure, so all of it is unit-tested.

import 'package:flutter/foundation.dart';

/// One selectable lane key, identified by its layer, property and comp frame
/// (the value identity egui's `LaneKeySel` carries).
@immutable
class LaneKeyId {
  final String layerId;
  final String property;
  final int frame;

  const LaneKeyId(this.layerId, this.property, this.frame);

  @override
  bool operator ==(Object other) =>
      other is LaneKeyId &&
      other.layerId == layerId &&
      other.property == property &&
      other.frame == frame;

  @override
  int get hashCode => Object.hash(layerId, property, frame);
}

/// Apply a modifier-aware click to the lane selection (egui note 2.6): a plain
/// click ([additive] false) replaces it with just [key]; an additive click
/// (Ctrl/Shift) toggles the key's membership. Mutates [selection] in place.
void laneSelectClick(
  Set<LaneKeyId> selection,
  LaneKeyId key, {
  required bool additive,
}) {
  if (additive) {
    if (!selection.remove(key)) selection.add(key);
  } else {
    selection
      ..clear()
      ..add(key);
  }
}

/// Group selected keys by their (layer, property) channel, each mapped to its
/// sorted frame list — the shape the drag commit walks so it fires one
/// `shiftKeyframes(layer, property, frames, delta)` per channel (one undo step
/// each), exactly as egui commits its lane drag per property.
Map<(String, String), List<int>> groupKeysForShift(Iterable<LaneKeyId> keys) {
  final out = <(String, String), List<int>>{};
  for (final k in keys) {
    (out[(k.layerId, k.property)] ??= <int>[]).add(k.frame);
  }
  for (final frames in out.values) {
    frames.sort();
  }
  return out;
}
