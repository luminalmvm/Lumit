// Normalise: bring a sound up, or down, so its loudest sample lands at the level
// Settings asks for. Every clip or layer by its own peak, as one undo step.
//
// The peaks are read first, because the first read of a file decodes it, and the
// writes all happen together once they are in.

import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/audio.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'layer_fold_frb.dart' show asOneUndoStep;

/// Picked clips, as the layer each is on and its id. [then] runs inside the undo
/// step, for a panel that writes something of its own after an edit. With
/// [loudness] each clip's measured loudness lands on [targetDb] in LUFS, in
/// place of its loudest sample.
Future<void> normaliseClips(
  LumitState app,
  double targetDb,
  Iterable<(LayerReference, UuidValue)> picked, {
  required void Function() then,
  bool loudness = false,
}) async {
  final byLayer = <LayerReference, List<UuidValue>>{};
  for (final (layer, clip) in picked) {
    (byLayer[layer] ??= []).add(clip);
  }
  final levels = <(LayerReference, BridgeClipLevel)>[];
  for (final MapEntry(key: layer, value: clips) in byLayer.entries) {
    try {
      for (final level in loudness
          ? await layer.loudnessClips(clips: clips, targetLufs: targetDb)
          : await layer.normaliseClips(clips: clips, targetDb: targetDb)) {
        levels.add((layer, level));
      }
    } catch (_) {
      // The layer has gone. The rest still take theirs.
    }
  }
  asOneUndoStep(app.project, () {
    _writeClips(levels);
    then();
  }, name: 'Normalise');
}

/// Picked clips' own level back at 0 dB, as one undo step. A clip already there
/// makes no change.
void resetClipGain(
  LumitState app,
  Iterable<(LayerReference, BridgeClip)> picked, {
  required void Function() then,
}) {
  asOneUndoStep(app.project, () {
    for (final (layer, clip) in picked) {
      if (clip.gainDb == 0) continue;
      try {
        layer.setClipGain(clip: clip.id, db: 0);
      } catch (_) {
        // A locked layer refuses, and the rest still take it.
      }
    }
    then();
  }, name: 'Reset gain');
}

void _writeClips(List<(LayerReference, BridgeClipLevel)> levels) {
  for (final (layer, level) in levels) {
    try {
      layer.setClipGain(clip: level.clip, db: level.db);
    } catch (_) {
      // A locked layer refuses, and the rest still take it.
    }
  }
}

/// Whole layers: every clip on a Sequence layer by its own peak, and a layer
/// that is one file through its Volume, which takes a key at [frame] when keyed.
Future<void> normaliseLayers(
  LumitState app,
  CompositionReference comp,
  int frame,
  double targetDb,
  List<BridgeLayerEntry> targets, {
  required void Function() then,
}) async {
  final levels = <(LayerReference, BridgeClipLevel)>[];
  final volumes = <(BridgeLayerEntry, double)>[];
  for (final t in targets) {
    try {
      if (t.info.clips.isNotEmpty) {
        final clips = [for (final c in t.info.clips) c.id];
        for (final level
            in await t.layer.normaliseClips(clips: clips, targetDb: targetDb)) {
          levels.add((t.layer, level));
        }
      } else if (await t.layer.normaliseLayer(targetDb: targetDb)
          case final db?) {
        volumes.add((t, db));
      }
    } catch (_) {
      // The layer has gone. The rest still take theirs.
    }
  }
  asOneUndoStep(app.project, () {
    _writeClips(levels);
    for (final (t, db) in volumes) {
      try {
        t.layer.setVolumeDb(
            value: scalarWithValueAt(t.info.volumeDb, db, comp, frame));
      } catch (_) {
        // A locked layer refuses, and the rest still take it.
      }
    }
    then();
  }, name: 'Normalise');
}
