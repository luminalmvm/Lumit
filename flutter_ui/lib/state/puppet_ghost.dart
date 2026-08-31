// Where the puppet's mesh actually is, at the frame on screen (K-704, PU3).
//
// In plain terms: the wireframe the Viewer draws under a puppet tool is the
// engine's own mesh, bent by the pins. It is never in the document and never in
// a project file — it is rebuilt from the layer's alpha inside the render — so
// the only honest way to draw it is to ask for the one the render just used. A
// second copy worked out here would drift from the picture the moment anything
// about the mesh changed.
//
// **Why this is a cache and not a call in the paint path.** The Viewer rebuilds
// on every movement of the pointer, and a bridge call per rebuild is what
// K-184's budget exists to stop (K-681 gates it at zero). The answer changes for
// exactly three reasons — a different layer, a new frame from the engine, an
// edit — and it is held against exactly those three, so a hover asks nothing.

import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

/// The mesh one layer is showing, held until something can have changed it.
class PuppetGhosts {
  BridgePuppetGhost? _ghost;

  UuidValue? _layer;
  int? _generation;
  BigInt? _revision;

  /// The wireframe to draw, or null when the layer has no mesh — nothing
  /// opaque to cut one from, or no puppet tool armed on it.
  BridgePuppetGhost? get ghost => _ghost;

  /// Bring the held answer up to date for [layer] at document [revision].
  ///
  /// [generation] is the engine's frame counter (`frameArrived`): the mesh moves
  /// when a *frame* lands, not when the document changes, so a playhead move —
  /// which touches neither the layer nor the revision — would otherwise leave
  /// the wireframe on the pose it had at the last edit.
  ///
  /// Cheap and call-free when none of the three has moved. A null [layer] —
  /// no puppet tool armed, or nothing selected — empties the copy rather than
  /// leaving the last layer's mesh over a picture it does not belong to.
  void refresh({
    required LayerReference? layer,
    required int generation,
    required BigInt? revision,
  }) {
    final id = layer?.internallayerId;
    if (_layer == id && _generation == generation && _revision == revision) {
      return;
    }
    _layer = id;
    _generation = generation;
    _revision = revision;
    if (layer == null) {
      _ghost = null;
      return;
    }
    try {
      _ghost = layer.puppetGhost();
    } catch (_) {
      // The layer went away between the selection and the ask. No wireframe is
      // the right answer, and it is not worth a dialogue.
      _ghost = null;
    }
  }
}
