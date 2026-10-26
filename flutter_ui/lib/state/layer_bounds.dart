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
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:lumit_flutter/src/rust/api/shape.dart';
import 'package:lumit_flutter/src/rust/api/wireframes.dart';
import 'package:lumit_flutter/panels/graph_maths.dart' show evaluateScalar;
import 'package:lumit_flutter/panels/layer_fold_frb.dart' show maxShapeCopies;
import 'package:lumit_flutter/panels/shape_entries.dart';
import 'package:uuid/uuid.dart';

/// A Null layer's box, in layer pixels.
///
/// A Null has no pixels at all — it exists to be parented to (docs/01) — so its
/// size is a drawing convention rather than a fact about content. 100×100 is
/// After Effects' own, and the transform the engine gives a new Null anchors on
/// the same square.
const Size nullLayerBounds = Size(100, 100);

/// Where [document]'s lines sit inside its layer. This is the engine's own
/// layout, asked of the bridge and remembered.
///
/// The box round the words, the caret between two letters and the letter a
/// click lands on are all read off the layout the rasteriser sets the block
/// with (`lumit_text::layout`), so none of them can drift from the picture.
/// Typing asks once per keystroke and painting asks every frame, so the
/// answers are held, and dropped wholesale once there are enough of them.
///
/// [text] is measured in place of the document's own words when given, which
/// is how a line still being typed is measured.
BridgeTextBlock measuredText(BridgeTextDocument document, {String? text}) {
  final key = (
    text ?? document.text,
    document.size,
    document.style,
    document.paragraph,
    textIsAnimated(document),
  );
  final held = _blocks[key];
  if (held != null) return held;
  if (_blocks.length >= _maxHeldBlocks) _blocks.clear();
  return _blocks[key] = measureText(
    text: key.$1,
    size: key.$2,
    style: key.$3,
    paragraph: key.$4,
    animated: key.$5,
  );
}

final Map<(String, double, BridgeTextStyle, BridgeParagraphStyle, bool),
    BridgeTextBlock> _blocks = {};
const int _maxHeldBlocks = 512;

/// A text layer's box, in layer pixels: exactly the raster the engine draws
/// the block into, so the wireframe hugs the words.
///
/// Empty text still gets a box, half a size wide and as tall as a line of
/// capitals, since the engine draws it as one transparent pixel. A layer
/// waiting to be typed into is then visible and says what size it will be
/// set at. With a box width it is as wide as the box the words will wrap to.
Size textLayerBounds(BridgeTextDocument document) {
  final block = measuredText(document);
  if (document.text.isEmpty) {
    return Size(document.paragraph.boxWidth ?? document.size * 0.5,
        block.lines.first.baseline);
  }
  return Size(block.width, block.height);
}

/// Whether the engine draws [document] into the bigger box animators get:
/// a straight line with at least one animator (a line on a path keeps the
/// path's own box either way).
bool textIsAnimated(BridgeTextDocument document) =>
    document.animators.isNotEmpty && document.path == null;

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
Rect? shapeContentsRect(List<BridgeShapeEntry> contents, {double t = 0}) {
  double? minX, minY, maxX, maxY;
  for (final (index, entry) in contents.indexed) {
    if (entry.kind is! BridgeShapeEntryKind_Item) continue;
    final item = (entry.kind as BridgeShapeEntryKind_Item).field0;
    final own = _boxOf(_controlPoints(item));
    if (own == null) continue;
    // The modifiers acting on this item, in the order they apply: the
    // engine's `grown`, then its copies.
    final mods = contents.modifiersOn(index);
    var box = own;
    var margin = 0.0;
    for (final m in mods) {
      switch (m.kind) {
        // A twist keeps the art inside the box it was turned in, as the
        // engine's `Twist::bounds` says.
        case BridgeShapeModifierKind_Twist(:final field0):
          box = _twistBox(item, field0, t);
        // A deform moves the outline no further than its amount:
        // `Deform::bounds`.
        case BridgeShapeModifierKind_Deform(field0: final deform):
          final amount = evaluateScalar(deform.amount, t).abs();
          if (deform.mode != 1) {
            box = (
              box.$1 - amount,
              box.$2 - amount,
              box.$3 + amount,
              box.$4 + amount
            );
          } else {
            final (mx, my) = ((own.$1 + own.$3) / 2, (own.$2 + own.$4) / 2);
            final grow = 1 + amount / 100;
            final rx =
                math.max((box.$1 - mx).abs(), (box.$3 - mx).abs()) * grow;
            final ry =
                math.max((box.$2 - my).abs(), (box.$4 - my).abs()) * grow;
            box = (mx - rx, my - ry, mx + rx, my + ry);
          }
        // An outline pushed *out* of the path sits further out. Pulled in it
        // never needs more room.
        case BridgeShapeModifierKind_Offset(:final field0):
          margin += math.max(0.0, evaluateScalar(field0, t));
        default:
          break;
      }
    }
    // Half the outline sits outside the path.
    final half =
        (item.stroke != null ? evaluateScalar(item.strokeWidth, t) / 2 : 0.0) +
            margin;
    final (ix0, iy0, ix1, iy1) =
        (box.$1 - half, box.$2 - half, box.$3 + half, box.$4 + half);
    // Every copy's box, unioned. One copy and the identity, every shape
    // nobody has repeated, is exactly the box above.
    for (final m in shapeCopyTransforms(mods, t)) {
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

/// Every vertex of [item]'s path and both its handles' ends.
Iterable<(double, double)> _controlPoints(BridgeShapeItem item) => [
      for (final v in item.vertices) ...[
        (v.x, v.y),
        (v.x + v.tanInX, v.y + v.tanInY),
        (v.x + v.tanOutX, v.y + v.tanOutY),
      ],
    ];

/// The box round [points] as (left, top, right, bottom), or null for none.
(double, double, double, double)? _boxOf(Iterable<(double, double)> points) {
  (double, double, double, double)? box;
  for (final (x, y) in points) {
    box = box == null
        ? (x, y, x, y)
        : (
            math.min(box.$1, x),
            math.min(box.$2, y),
            math.max(box.$3, x),
            math.max(box.$4, y),
          );
  }
  return box;
}

/// The box a twisted [item] stays inside: how far its control points reach
/// along the axis from the centre, and across it either way. The engine's
/// `Twist::bounds`.
(double, double, double, double) _twistBox(
    BridgeShapeItem item, BridgeShapeTwist twist, double t) {
  final turn = evaluateScalar(twist.axis, t) * math.pi / 180;
  final (ax, ay) = (math.cos(turn), math.sin(turn));
  final cx = evaluateScalar(twist.centreX, t);
  final cy = evaluateScalar(twist.centreY, t);
  var (back, forward, across) = (0.0, 0.0, 0.0);
  for (final (x, y) in _controlPoints(item)) {
    final along = (x - cx) * ax + (y - cy) * ay;
    final side = -(x - cx) * ay + (y - cy) * ax;
    back = math.min(back, along);
    forward = math.max(forward, along);
    across = math.max(across, side.abs());
  }
  return _boxOf([
    for (final along in [back, forward])
      for (final side in [-across, across])
        (cx + along * ax - side * ay, cy + along * ay + side * ax),
  ])!;
}

/// The size of that box, floored at a pixel each way so a straight line still
/// has a layer to be drawn on.
Size? shapeContentsBounds(List<BridgeShapeEntry> contents, {double t = 0}) {
  final rect = shapeContentsRect(contents, t: t);
  if (rect == null) return null;
  return Size(math.max(rect.width, 1), math.max(rect.height, 1));
}

/// The transform each copy of a piece of art is drawn with at [t], given the
/// modifiers [mods] acting on it, as `[a, b, c, d, e, f]` mapping `(x, y)` to
/// `(ax + cy + e, bx + dy + f)`.
///
/// The engine's `copies_at` in the same six numbers, and it has to stay that
/// way: the engine sizes the raster from its version and the wireframe is
/// drawn from this one. Stacked repeaters multiply, every copy of the later
/// one made of every copy of the earlier, held to the engine's thousand. A
/// single identity, no repeater, is what every shape answers until somebody
/// asks for more.
List<List<double>> shapeCopyTransforms(
    List<BridgeShapeModifier> mods, double t) {
  var out = <List<double>>[identityTransform];
  for (final m in mods) {
    if (m.kind case BridgeShapeModifierKind_Repeat(:final field0)) {
      final copies = _repeaterCopies(field0, t);
      if (copies.length <= 1) continue;
      out = [
        for (final later in copies)
          for (final earlier in out) _then(earlier, later),
      ];
      if (out.length > maxShapeCopiesDrawn) {
        out = out.sublist(0, maxShapeCopiesDrawn);
      }
    }
  }
  return out;
}

/// The most copies stacked repeaters draw of one piece of art between them:
/// `MAX_COPIES_DRAWN` in the engine.
const int maxShapeCopiesDrawn = 1000;

/// The copies one repeater draws at [t]: the engine's `Repeater::copies_at`.
List<List<double>> _repeaterCopies(BridgeShapeRepeater item, double t) {
  final copies = evaluateScalar(item.copies, t)
      .round()
      .clamp(1, maxShapeCopies.toInt());
  if (copies <= 1) return const [identityTransform];
  final offset = evaluateScalar(item.offset, t)
      .round()
      .clamp(-maxShapeCopies.toInt(), maxShapeCopies.toInt());

  final rotation = evaluateScalar(item.rotation, t) * math.pi / 180;
  final scale = evaluateScalar(item.scale, t) / 100;
  final ax = evaluateScalar(item.anchorX, t);
  final ay = evaluateScalar(item.anchorY, t);
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
    ax - (a * ax + c * ay) + evaluateScalar(item.positionX, t),
    ay - (b * ax + d * ay) + evaluateScalar(item.positionY, t),
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

  /// The probed sizes, in the shape the engine's wireframe call takes: its own
  /// probe lives on the worker and that call runs on the caller's thread, so
  /// what is already measured here goes along rather than being asked for
  /// again. An item nothing has probed yet is simply absent, and the engine
  /// falls back to the comp's size for it.
  List<BridgeMediaSize> get mediaSizes => [
        for (final e in _media.entries)
          BridgeMediaSize(
            item: e.key,
            width: e.value.width.round(),
            height: e.value.height.round(),
          ),
      ];

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

    // Text measures its own line, exactly as the engine lays it out.
    if (entry.info.kind == BridgeLayerKind.text) {
      try {
        final document = entry.layer.getText();
        if (document != null) {
          return textLayerBounds(document);
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
