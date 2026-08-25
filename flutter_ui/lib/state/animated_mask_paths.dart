// Where a keyed mask's shape actually is, at the frame on screen (K-342).
//
// In plain terms: once a mask's path is animated, the shape stored on the mask
// is no longer the shape the picture shows — the picture interpolates between
// the keyed shapes, and the stored one is only what the drawing tools last
// wrote. The wireframe drew the stored one, so a mask that animated correctly
// in the render appeared to snap back to where it started the moment the drag
// ended.
//
// Asking the engine is the only honest answer: interpolating two paths means
// reconciling their vertex counts by splitting cubics (K-339), and a second
// copy of that here would drift from the one that draws the pixels.
//
// **Why this is a cache and not a call in the paint path.** The Viewer rebuilds
// on every movement of the pointer, and a bridge call per rebuild is what
// K-184's budget exists to stop. The answer only changes when the document
// changes or the playhead moves, so it is held against exactly those two and a
// hover asks nothing.

import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

/// The evaluated shapes of a composition's animated masks, held per document
/// revision and per frame.
class AnimatedMaskPaths {
  /// Vertices by layer id, then mask id — empty, which is the ordinary case,
  /// meaning no mask on the composition is animated.
  Map<UuidValue, Map<UuidValue, List<BridgeVertex>>> _byLayer = const {};

  BigInt? _revision;
  int? _frame;
  UuidValue? _comp;

  /// The path [mask] on [layer] is showing, or null when it is not animated —
  /// in which case the mask's own vertices are already right.
  List<BridgeVertex>? pathOf(UuidValue layer, UuidValue mask) =>
      _byLayer[layer]?[mask];

  /// Bring the held answer up to date for [comp] at [frame] and document
  /// [revision]. Cheap and call-free when none of the three has moved.
  void refresh({
    required CompositionReference comp,
    required int frame,
    required BigInt? revision,
  }) {
    final id = comp.internalid;
    if (_comp == id && _frame == frame && _revision == revision) return;
    _comp = id;
    _frame = frame;
    _revision = revision;
    try {
      final rows = comp.animatedMaskPathsAt(frame: frame);
      if (rows.isEmpty) {
        _byLayer = const {};
        return;
      }
      final next = <UuidValue, Map<UuidValue, List<BridgeVertex>>>{};
      for (final row in rows) {
        (next[row.layer] ??= {})[row.mask] = row.vertices;
      }
      _byLayer = next;
    } catch (_) {
      // The comp went away, or the frame is one the rate cannot name. The
      // wireframe falls back to the stored path, which is where it was before
      // any of this existed.
      _byLayer = const {};
    }
  }
}
