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

import 'package:lumit_flutter/src/rust/api/layer.dart';
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
