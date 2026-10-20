// How each Viewer's picture sees its comp's 3D space, held between rebuilds.
//
// A 3D layer's box is drawn through the camera the picture is drawn through,
// and the engine is the one that knows where that camera is: a keyed one, a
// two-node one aimed at its point of interest, or one following a solve. So
// the engine is asked, and only when the answer could have changed.

import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

/// The rows that draw a 3D layer flat, as the compositor does with no camera.
const List<double> flatView = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 1];

/// The engine's view rows per Viewer, asked again only when the comp, the
/// frame, the document, a solve or the view itself has moved.
class ViewProjections {
  final Map<int, _Held> _byView = {};

  /// Comp space to the picture, three rows of four, for [viewId] showing
  /// [comp] at [frame] through [view] (null for the comp's own camera). Flat
  /// while the engine cannot say.
  List<double> of({
    required CompositionReference comp,
    required int viewId,
    required int frame,
    required BigInt? revision,
    required int generation,
    required BridgeCameraPose? view,
  }) {
    final key = (comp.internalid, frame, revision, generation, view);
    final held = _byView[viewId];
    if (held != null && held.key == key) return held.rows;
    var rows = flatView;
    try {
      rows = comp.viewProjection(frame: BigInt.from(frame), view: view);
    } catch (_) {
      // No worker yet, or a comp that has gone. Asked again once the frame,
      // the document or the view moves.
    }
    _byView[viewId] = _Held(key, rows);
    return rows;
  }

  /// Let go of a closed view's rows.
  void forget(int viewId) => _byView.remove(viewId);
}

class _Held {
  final Object key;
  final List<double> rows;
  const _Held(this.key, this.rows);
}

/// The engine's pose of each camera layer, aim and solve included, asked
/// again only when the frame, the document or a solve has moved. Only a
/// selected camera's handles ask, so a comp with cameras costs nothing until
/// one is picked.
class CameraPoses {
  final Map<Object, (Object, BridgeCameraPose?)> _byLayer = {};

  BridgeCameraPose? of(
    LayerReference layer, {
    required Object id,
    required int frame,
    required BigInt? revision,
    required int generation,
  }) {
    final key = (frame, revision, generation);
    final held = _byLayer[id];
    if (held != null && held.$1 == key) return held.$2;
    BridgeCameraPose? pose;
    try {
      pose = layer.cameraPoseAt(frame: BigInt.from(frame));
    } catch (_) {
      // A camera that has gone. Asked again once the document moves.
    }
    _byLayer[id] = (key, pose);
    return pose;
  }
}
