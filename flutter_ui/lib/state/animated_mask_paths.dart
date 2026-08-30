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

  /// What [refresh] was last told about whether anything drawn is animated.
  /// Part of the memo key, not just a guard: selecting a layer with a keyed
  /// mask flips it true without moving the frame or the document, and a memo
  /// that ignored it would hold the empty answer and draw the stored path.
  bool _animated = false;

  /// The path [mask] on [layer] is showing, or null when it is not animated —
  /// in which case the mask's own vertices are already right.
  List<BridgeVertex>? pathOf(UuidValue layer, UuidValue mask) =>
      _byLayer[layer]?[mask];

  /// Bring the held answer up to date for [comp] at [frame] and document
  /// [revision]. Cheap and call-free when none of the three has moved.
  ///
  /// [anyAnimated] is "a mask whose shape is actually drawn carries path
  /// keys", which the caller reads off the read model and the selection for
  /// nothing. When it is false there is no interpolated shape to draw at *any*
  /// frame — a mask with no path keys is never listed whatever frame is asked
  /// for, and a mask on a layer nobody outlines is never painted — so the ask
  /// is skipped rather than made once a frame (ui-performance §4.5).
  ///
  /// Passing it in rather than guarding the call from outside is what keeps
  /// the held copy honest: when it goes false — the last path key deleted, or
  /// the layer deselected — the copy is emptied, instead of a mask carrying on
  /// with the interpolated shape it no longer has.
  void refresh({
    required CompositionReference comp,
    required int frame,
    required BigInt? revision,
    required bool anyAnimated,
  }) {
    final id = comp.internalid;
    if (_comp == id &&
        _frame == frame &&
        _revision == revision &&
        _animated == anyAnimated) {
      return;
    }
    _comp = id;
    _frame = frame;
    _revision = revision;
    _animated = anyAnimated;
    if (!anyAnimated) {
      _byLayer = const {};
      return;
    }
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
