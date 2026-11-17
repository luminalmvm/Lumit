// Normalise: bring a sound up, or down, so its loudest sample lands at the level
// Settings asks for. Every clip or layer by its own peak, as one undo step.
//
// The peaks are read first, because the first read of a file decodes it, and the
// writes all happen together once they are in. Normalise together moves them
// all by one change instead, so their balance holds.

import 'dart:math' show min;

import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/audio.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

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
/// that is one file through its Gain, so its Volume and keys stay as they are.
/// The engine reads the file's own peak, so the answer is the whole Gain.
Future<void> normaliseLayers(
  LumitState app,
  CompositionReference comp,
  int frame,
  double targetDb,
  List<BridgeLayerEntry> targets, {
  required void Function() then,
}) async {
  final levels = <(LayerReference, BridgeClipLevel)>[];
  final gains = <(BridgeLayerEntry, double)>[];
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
        gains.add((t, db));
      }
    } catch (_) {
      // The layer has gone. The rest still take theirs.
    }
  }
  asOneUndoStep(app.project, () {
    _writeClips(levels);
    for (final (t, db) in gains) {
      try {
        t.layer.setAudioGainDb(db: db);
      } catch (_) {
        // A locked layer refuses, and the rest still take it.
      }
    }
    then();
  }, name: 'Normalise');
}

/// Several clips or layers brought up or down by one change, so the loudest
/// lands on [targetDb] and the rest keep their balance: by peak, or with
/// [loudness] by measured loudness in LUFS. A layer goes by its clips, or by
/// its Gain where it is one file (by peak only). One undo step.
Future<void> normaliseTogether(
  LumitState app,
  double targetDb, {
  List<(LayerReference, BridgeClip)> clips = const [],
  List<BridgeLayerEntry> layers = const [],
  required void Function() then,
  bool loudness = false,
}) async {
  final byLayer = <LayerReference, List<BridgeClip>>{};
  for (final (layer, clip) in [
    ...clips,
    for (final t in layers)
      for (final c in t.info.clips) (t.layer, c),
  ]) {
    (byLayer[layer] ??= []).add(clip);
  }
  // What each would move on its own, and how it is written moved by [by].
  final moves = <(double, void Function(double by))>[];
  for (final MapEntry(key: layer, value: picked) in byLayer.entries) {
    final now = {for (final c in picked) c.id: c.gainDb};
    final ids = [for (final c in picked) c.id];
    try {
      for (final level in loudness
          ? await layer.loudnessClips(clips: ids, targetLufs: targetDb)
          : await layer.normaliseClips(clips: ids, targetDb: targetDb)) {
        final was = now[level.clip] ?? 0;
        moves.add((
          level.db - was,
          (by) => layer.setClipGain(clip: level.clip, db: was + by),
        ));
      }
    } catch (_) {
      // The layer has gone. The rest still take theirs.
    }
  }
  for (final t in layers) {
    if (loudness || t.info.clips.isNotEmpty) continue;
    try {
      if (await t.layer.normaliseLayer(targetDb: targetDb) case final db?) {
        final was = t.info.audioGainDb;
        moves.add((db - was, (by) => t.layer.setAudioGainDb(db: was + by)));
      }
    } catch (_) {
      // The layer has gone. The rest still take theirs.
    }
  }
  if (moves.isEmpty) return;
  // The loudest needs the least, and every one moves by that.
  final by = moves.map((m) => m.$1).reduce(min);
  asOneUndoStep(app.project, () {
    for (final (_, write) in moves) {
      try {
        write(by);
      } catch (_) {
        // A locked layer refuses, and the rest still take it.
      }
    }
    then();
  }, name: 'Normalise');
}
