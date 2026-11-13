// A keyed layer's motion path, as the engine samples it (docs/07 §2.4).
//
// In plain terms: once a layer's Position is animated, the line it travels
// along is worth seeing on the picture, with a dot at every key. The engine
// samples that line once per frame across the keyed range, so what is drawn is
// the curve the render follows and not a second guess at it.
//
// **Why this is a cache and not a call in the paint path.** The Viewer rebuilds
// on every movement of the pointer, and a bridge call per rebuild is what the
// rebuild budget exists to stop. A path changes only when the document does,
// and is drawn only for the outlined layers, so it is asked for once per
// outlined layer per document revision and held.

import 'dart:typed_data';

import 'package:lumit_flutter/panels/graph_maths.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/lib.dart';
import 'package:uuid/uuid.dart';

/// The motion paths of the layers being drawn, held per document revision.
class MotionPaths {
  /// By layer id. A null value is an answer too: the layer's Position is
  /// still, so there is no path to draw and nothing to ask again.
  final Map<UuidValue, BridgeMotionPath?> _byLayer = {};

  int? _revision;

  /// The path [layer] is following, or null when its Position is still.
  ///
  /// [revision] is the document revision the caller is drawing; a move of it
  /// empties everything held, and the first ask after it goes to the engine.
  BridgeMotionPath? pathOf(LayerReference layer, {required int? revision}) {
    if (revision != _revision) {
      _revision = revision;
      _byLayer.clear();
    }
    final id = layer.internallayerId;
    if (_byLayer.containsKey(id)) return _byLayer[id];
    BridgeMotionPath? path;
    try {
      path = layer.motionPath();
    } catch (_) {
      // The layer went away between the model being read and this ask. No
      // path, and no asking again until the document moves.
    }
    _byLayer[id] = path;
    return path;
  }
}

/// How far one axis' handle sits from key [i] on one side, in that axis' own
/// units, or null where that side has no span to draw one across — the
/// engine's `reach` in `motion_path.rs`, ported so a drag can redraw it.
double? _reach(List<BridgeKeyframe> keys, int i, {required bool out}) {
  final j = out ? i + 1 : i - 1;
  if (j < 0 || j >= keys.length) return null;
  final dt =
      (rationalSeconds(keys[j].time) - rationalSeconds(keys[i].time)).abs();
  return switch (resolvedSide(keys, i, isOut: out)) {
    BridgeSideInterp_Bezier(:final field0) => (out ? 1 : -1) *
        field0.speed *
        field0.influence.clamp(minTangentReach, maxInfluence) *
        dt,
    BridgeSideInterp_Linear() => (keys[j].value - keys[i].value) / 3,
    _ => null,
  };
}

/// [path] as it is with Position's axes set to [x] and [y]: the line a drag
/// in flight is making, sampled here at the engine's own frames because the
/// engine's path only catches up when the document does. [fps] is the comp's
/// rate. An axis an expression drives keeps the engine's samples, since only
/// the engine can run one.
BridgeMotionPath motionPathWith(
    BridgeMotionPath path, BridgeScalar x, BridgeScalar y, double fps) {
  final ownX = x is! BridgeScalar_Expression && fps > 0;
  final ownY = y is! BridgeScalar_Expression && fps > 0;
  final samples = Float64List.fromList(path.samples);
  for (var i = 0; i + 1 < samples.length; i += 2) {
    final t = (path.firstFrame + i ~/ 2) / fps;
    if (ownX) samples[i] = evaluateScalar(x, t);
    if (ownY) samples[i + 1] = evaluateScalar(y, t);
  }
  final (kx, ky) = (keysOf(x), keysOf(y));
  BridgeMotionKey moved(BridgeMotionKey key) {
    final t = rationalSeconds(key.time);
    final at = (
      ownX ? evaluateScalar(x, t) : key.x,
      ownY ? evaluateScalar(y, t) : key.y,
    );
    F64Array2? handle(F64Array2? was, {required bool out}) {
      final (ix, iy) = (key.xIndex, key.yIndex);
      if (was == null || ix == null || iy == null) return was;
      if (ix >= kx.length || iy >= ky.length) return was;
      final (rx, ry) = (_reach(kx, ix, out: out), _reach(ky, iy, out: out));
      if (rx == null || ry == null) return was;
      return F64Array2(Float64List.fromList([at.$1 + rx, at.$2 + ry]));
    }

    return BridgeMotionKey(
      time: key.time,
      frame: key.frame,
      x: at.$1,
      y: at.$2,
      z: key.z,
      xIndex: key.xIndex,
      yIndex: key.yIndex,
      handleIn: handle(key.handleIn, out: false),
      handleOut: handle(key.handleOut, out: true),
    );
  }

  return BridgeMotionPath(
    firstFrame: path.firstFrame,
    samples: samples,
    depths: path.depths,
    keys: [for (final key in path.keys) moved(key)],
  );
}
