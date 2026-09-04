// How big a layer is, in its own pixels — the rectangle the Viewer draws a
// wireframe round and hit-tests a click against.
//
// **In plain terms.** A layer's transform says where it sits, how big it is
// drawn and which way up. It does not say how big the *thing* is: that comes
// from what the layer is made of — a clip's video is 1920×1080, a solid is
// whatever it was made at, a precomp is the size of the comp inside it. Without
// that, "draw a box round this layer" and "is the pointer over this layer?"
// have no answer.
//
// **Why the answers are cached.** A clip's size is a question about a *file*,
// and the only honest way to answer it is to open the file and look — which is
// disk work, and asynchronous. So each footage item is probed once and
// remembered for the session; everything else is a cheap read of the document
// and is remembered for as long as the document does not move. Nothing here
// blocks a paint: while a probe is in flight the layer falls back to the comp's
// own size, and the answer arriving repaints whoever is listening.

import 'dart:math' as math;
import 'dart:ui' show Rect, Size;

import 'package:flutter/foundation.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/panels/graph_maths.dart' show evaluateScalar;
import 'package:lumit_flutter/panels/layer_fold_frb.dart' show maxShapeCopies;
import 'package:uuid/uuid.dart';

/// A Null layer's box, in layer pixels.
///
/// A Null has no pixels at all — it exists to be parented to (docs/01) — so its
/// size is a drawing convention rather than a fact about content. 100×100 is
/// After Effects' own, and the transform the engine gives a new Null anchors on
/// the same square.
const Size nullLayerBounds = Size(100, 100);

/// How wide a line of text is, roughly, in layer pixels.
///
/// **This is the engine's own estimate**, mirrored here on purpose: the bridge
/// anchors a text layer at half of `characters × size × 0.5`, and the caret and
/// the box are placed by the same sum. None of them is the true advance width
/// of the glyphs — that is known only to the rasteriser — but all of them being
/// wrong the same way is what keeps the caret, the box and the picture from
/// disagreeing about where the line ends.
double estimatedTextWidth(String text, double size) =>
    text.runes.length * size * 0.5;

/// A text layer's box, in layer pixels.
///
/// **The height is the point size and nothing more.** It used to be the whole
/// composition — text had no measured bounds on this frontend, and the comp was
/// the fallback — so a click with the Type tool put a box the size of the frame
/// round a line of 12-pixel text, and the wireframe said nothing about where the
/// words were.
///
/// An empty line still gets a box: one character's worth of width, so a layer
/// waiting to be typed into is visible and the box says what size it will be
/// set at rather than vanishing.
Size textLayerBounds(String text, double size) => Size(
      text.isEmpty ? size * 0.5 : estimatedTextWidth(text, size),
      size,
    );

/// The box a shape layer's art fills, in the **art's** own coordinates, or null
/// when there is no art.
///
/// The **control points** bound the curve rather than the curve itself — a cubic
/// never leaves its own control hull — which is the same rule `lumit-core`'s
/// `shape::ShapeItem::bounds` follows. The two must agree: the engine sizes the
/// raster with its version and the wireframe is drawn from this one.
///
/// **Art coordinates are not layer pixels**. The engine draws a shape
/// layer's picture as exactly this box, so the layer's pixel (0, 0) is this
/// box's top-left corner — a vertex at art (x, y) is at layer pixel
/// (x − left, y − top). [shapeContentsBounds] answers the size, which is all a
/// wireframe box needs; anything drawing the *points* needs the corner too.
/// **The repeater's copies are part of the box**, so it is measured at
/// a time: a keyed repeater puts its copies somewhere new each frame. [t] is
/// seconds on the composition's clock, which is the clock the bridge already
/// hands a shape item's keys over on.
Rect? shapeContentsRect(List<BridgeShapeItem> contents, {double t = 0}) {
  double? minX, minY, maxX, maxY;
  for (final item in contents) {
    // Half the outline sits outside the path, and an outline pushed *out* of
    // it sits further out still. Pulled in it never needs more room.
    final half = (item.stroke != null ? item.strokeWidth / 2 : 0.0) +
        math.max(0.0, evaluateScalar(item.offsetAmount, t));
    double? ix0, iy0, ix1, iy1;
    for (final v in item.vertices) {
      for (final (x, y) in [
        (v.x, v.y),
        (v.x + v.tanInX, v.y + v.tanInY),
        (v.x + v.tanOutX, v.y + v.tanOutY),
      ]) {
        ix0 = ix0 == null ? x - half : math.min(ix0, x - half);
        iy0 = iy0 == null ? y - half : math.min(iy0, y - half);
        ix1 = ix1 == null ? x + half : math.max(ix1, x + half);
        iy1 = iy1 == null ? y + half : math.max(iy1, y + half);
      }
    }
    if (ix0 == null || iy0 == null || ix1 == null || iy1 == null) continue;
    // Every copy's box, unioned. One copy and the identity — every shape
    // nobody has repeated — is exactly the box above.
    for (final m in shapeCopyTransforms(item, t)) {
      for (final (x, y) in [
        (ix0, iy0),
        (ix1, iy0),
        (ix1, iy1),
        (ix0, iy1),
      ]) {
        final px = m[0] * x + m[2] * y + m[4];
        final py = m[1] * x + m[3] * y + m[5];
        minX = minX == null ? px : math.min(minX, px);
        minY = minY == null ? py : math.min(minY, py);
        maxX = maxX == null ? px : math.max(maxX, px);
        maxY = maxY == null ? py : math.max(maxY, py);
      }
    }
  }
  if (minX == null || minY == null || maxX == null || maxY == null) return null;
  return Rect.fromLTRB(minX, minY, maxX, maxY);
}

/// The size of that box, floored at a pixel each way so a straight line still
/// has a layer to be drawn on.
Size? shapeContentsBounds(List<BridgeShapeItem> contents, {double t = 0}) {
  final rect = shapeContentsRect(contents, t: t);
  if (rect == null) return null;
  return Size(math.max(rect.width, 1), math.max(rect.height, 1));
}

/// The transform each of [item]'s repeated copies is drawn with at [t], as
/// `[a, b, c, d, e, f]` mapping `(x, y)` to `(ax + cy + e, bx + dy + f)`.
///
/// The engine's `ShapeItem::copies_at` in the same six numbers, and it has to
/// stay that way: the engine sizes the raster from its version and the
/// wireframe is drawn from this one. A single identity — one copy, no repeater
/// — is what every shape answers until somebody asks for more.
List<List<double>> shapeCopyTransforms(BridgeShapeItem item, double t) {
  final copies = evaluateScalar(item.repeatCopies, t)
      .round()
      .clamp(1, maxShapeCopies.toInt());
  if (copies <= 1) return const [identityTransform];
  final offset = evaluateScalar(item.repeatOffset, t)
      .round()
      .clamp(-maxShapeCopies.toInt(), maxShapeCopies.toInt());

  final rotation = evaluateScalar(item.repeatRotation, t) * math.pi / 180;
  final scale = evaluateScalar(item.repeatScale, t) / 100;
  final ax = evaluateScalar(item.repeatAnchorX, t);
  final ay = evaluateScalar(item.repeatAnchorY, t);
  final a = math.cos(rotation) * scale;
  final b = math.sin(rotation) * scale;
  final c = -math.sin(rotation) * scale;
  final d = math.cos(rotation) * scale;
  // Move to the anchor, turn and scale there, move back, then translate.
  final step = <double>[
    a,
    b,
    c,
    d,
    ax - (a * ax + c * ay) + evaluateScalar(item.repeatPositionX, t),
    ay - (b * ax + d * ay) + evaluateScalar(item.repeatPositionY, t),
  ];
  final back = _inverse(step);

  var m = identityTransform.toList();
  for (var i = 0; i < offset.abs(); i++) {
    m = _then(m, offset >= 0 ? step : back);
  }
  final out = <List<double>>[];
  for (var j = 0; j < copies; j++) {
    out.add(m);
    m = _then(m, step);
  }
  return out;
}

/// The transform that changes nothing.
const List<double> identityTransform = [1, 0, 0, 1, 0, 0];

/// [m] followed by [n].
List<double> _then(List<double> m, List<double> n) => [
      m[0] * n[0] + m[1] * n[2],
      m[0] * n[1] + m[1] * n[3],
      m[2] * n[0] + m[3] * n[2],
      m[2] * n[1] + m[3] * n[3],
      m[4] * n[0] + m[5] * n[2] + n[4],
      m[4] * n[1] + m[5] * n[3] + n[5],
    ];

/// The transform that undoes [m], or the identity where it cannot be undone -
/// a copy scaled to nothing has no way back, and an identity is the answer
/// that draws something rather than dividing by zero.
List<double> _inverse(List<double> m) {
  final det = m[0] * m[3] - m[1] * m[2];
  if (det.abs() < 1e-12) return identityTransform.toList();
  final inv = 1 / det;
  final a = m[3] * inv;
  final b = -m[1] * inv;
  final c = -m[2] * inv;
  final d = m[0] * inv;
  return [a, b, c, d, -(m[4] * a + m[5] * c), -(m[4] * b + m[5] * d)];
}

/// Every layer's own size, answered from the document and remembered.
class LayerBoundsCache extends ChangeNotifier {
  /// Sizes by layer id, good for as long as [_revision] is.
  final Map<UuidValue, Size> _byLayer = {};

  /// Probed media sizes by footage item id. Kept for the session: a file's
  /// dimensions do not change under us, and a relink refreshes through the
  /// document revision below anyway.
  final Map<UuidValue, Size> _media = {};

  /// Footage items with a probe in flight, so a repaint does not start a
  /// second one.
  final Set<UuidValue> _probing = {};

  BigInt? _revision;

  /// Forget the per-layer answers when the document has moved on.
  ///
  /// The probed media sizes survive: what a *file* measures does not depend on
  /// the document, and re-probing on every edit would put FFmpeg in the paint
  /// path.
  void _atRevision(BigInt? revision) {
    if (revision == _revision) return;
    _revision = revision;
    _byLayer.clear();
  }

  /// The size of [entry]'s content in layer pixels, at document [revision].
  ///
  /// Never null and never zero: a layer whose real size is not knowable yet —
  /// a clip still being probed, a kind with no content of its own — measures
  /// the comp, which is the same fallback the engine uses when it places a clip
  /// it cannot probe.
  Size boundsOf(
    BridgeLayerEntry entry, {
    required BridgeCompSize compSize,
    required BigInt? revision,
    double t = 0,
  }) {
    _atRevision(revision);
    final id = entry.layer.internallayerId;
    // A shape layer is measured fresh every time: its box can move with the
    // *playhead* as well as with the document, and a cache keyed on the
    // revision alone would hand back yesterday's box while a keyed repeater
    // played. Measuring one is a walk over its own points, which is what this
    // cache exists to keep media probes out of, not arithmetic.
    if (entry.info.kind == BridgeLayerKind.shape) {
      return _measure(entry, compSize, t);
    }
    final held = _byLayer[id];
    if (held != null) return held;
    final measured = _measure(entry, compSize, t);
    _byLayer[id] = measured;
    return measured;
  }

  Size _compSize(BridgeCompSize s) =>
      Size(s.width.toDouble(), s.height.toDouble());

  Size _measure(BridgeLayerEntry entry, BridgeCompSize compSize, double t) {
    // A Null never draws, so its box is the convention above rather than
    // anything read from the document.
    if (entry.info.kind == BridgeLayerKind.nullLayer) return nullLayerBounds;

    // A shape layer is exactly as big as its art, and **that changes as the art
    // is edited** — the first kind whose size is not fixed by a source.
    // The cache follows the document's revision, so it keeps up; this comment
    // is here because the rest of this file was written when "a layer's size"
    // was a constant.
    if (entry.info.kind == BridgeLayerKind.shape) {
      final art = shapeContentsBounds(entry.info.shapeContents, t: t);
      return art ?? _compSize(compSize);
    }

    // Text measures its own line: the point size tall, and as wide as
    // the engine's estimate of the glyphs makes it.
    if (entry.info.kind == BridgeLayerKind.text) {
      try {
        final document = entry.layer.getText();
        if (document != null) {
          return textLayerBounds(document.text, document.size);
        }
      } catch (_) {
        // The layer went away between the model being read and this call.
      }
      return _compSize(compSize);
    }

    // **All three of these come off the read model**, which is what
    // makes an edit cheap: this cache is emptied on every document revision, so
    // asking the engine per layer here cost a `get_source_item` — and a
    // `get_size` or a `get_definition` behind it — for every layer on screen,
    // the moment any switch anywhere was clicked
    // (docs/impl/ui-performance.md §4.5).
    switch (entry.info.source) {
      // A nested comp is exactly as big as the comp inside it; a solid is the
      // size it was made at. The engine's own walk reads both into the model.
      case ItemReference_Composition() || ItemReference_Solid():
        final size = entry.info.sourceSize;
        if (size == null) return _compSize(compSize);
        return Size(size.width.toDouble(), size.height.toDouble());
      case ItemReference_Footage(:final field0):
        final item = field0.internalid;
        final probed = _media[item];
        if (probed != null) return probed;
        _probe(field0, item);
        return _compSize(compSize);
      // Text, Sequence, Adjustment, Camera and anything sourceless: the comp.
      // An adjustment layer genuinely is comp-sized (it is a container for
      // effects over everything below it); text has no measured bounds on this
      // frontend yet, and guessing a smaller box would put the handles
      // somewhere the glyphs are not.
      case _:
        return _compSize(compSize);
    }
  }

  /// Ask the engine for a clip's real dimensions, once.
  ///
  /// Fire-and-forget by design: the caller is painting, and the answer landing
  /// notifies listeners so the boxes are redrawn with it. A file that cannot be
  /// probed (missing, unreadable, audio-only) records nothing, so the layer
  /// keeps the comp-sized fallback rather than a box of zero pixels.
  void _probe(FootageReference footage, UuidValue item) {
    if (!_probing.add(item)) return;
    footage.mediaInfo().then((info) {
      _probing.remove(item);
      if (info == null || info.width <= 0 || info.height <= 0) return;
      _media[item] = Size(info.width.toDouble(), info.height.toDouble());
      // The per-layer answers were computed against the fallback.
      _byLayer.clear();
      notifyListeners();
    }, onError: (_) {
      _probing.remove(item);
    });
  }
}
