// The interpolation-coded keyframe glyph: the shape a key draws with and the
// painter that draws it. Ported faithfully from the egui graph editor
// (crates/lumit-ui/src/shell/graph.rs `key_shape`) and the lane drawer
// (inspector/lane.rs `draw_key_diamonds`/`draw_lane_glyph`), so a property's
// keys read the same in both frontends.
//
// In plain terms: a keyframe's corner "shape" tells you at a glance how it eases.
// A plain linear key is a diamond; a held key (a hard step) is a square; a
// bezier-eased key is a circle. Hold wins over bezier — a held key never eases
// out visually, so it always reads as a square.

import 'package:flutter/widgets.dart';

import '../../bridge/bridge.dart';

/// The three key glyph shapes, matching egui's `KeyShape`.
enum KeyShape { square, diamond, circle }

/// The glyph shape for a key with the given side-interpolation names (the
/// engine's `SideInterp` variants: `Hold`, `Linear`, `Bezier`). Hold on either
/// side codes a square; failing that, Bezier on either side codes a circle;
/// otherwise (Linear both sides) a diamond — exactly `graph.rs::key_shape`.
KeyShape keyShapeFor(String interpIn, String interpOut) {
  if (interpIn == 'Hold' || interpOut == 'Hold') return KeyShape.square;
  if (interpIn == 'Bezier' || interpOut == 'Bezier') return KeyShape.circle;
  return KeyShape.diamond;
}

/// The glyph shape of a bridge keyframe.
KeyShape keyShapeOf(BridgeKeyframe k) => keyShapeFor(k.interpIn, k.interpOut);

/// Draw one keyframe glyph centred at [pos]. When [selected] an accent ring
/// surrounds it (the lane selection); [fill] is the glyph body (accent normally,
/// a brighter tone when hot/dragged) and [outline] its 1 px edge. The sizes
/// mirror inspector/lane.rs so the two frontends paint the same marks.
void drawKeyGlyph(
  Canvas canvas,
  Offset pos,
  KeyShape shape, {
  required Color fill,
  required Color outline,
  bool selected = false,
  Color? selectRing,
}) {
  if (selected && selectRing != null) {
    canvas.drawCircle(
      pos,
      6,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.5
        ..color = selectRing,
    );
  }
  final body = Paint()..color = fill;
  final edge = Paint()
    ..style = PaintingStyle.stroke
    ..strokeWidth = 1
    ..color = outline;
  switch (shape) {
    case KeyShape.square:
      final r = Rect.fromCenter(center: pos, width: 6.5, height: 6.5);
      final rr = RRect.fromRectAndRadius(r, const Radius.circular(1));
      canvas.drawRRect(rr, body);
      canvas.drawRRect(rr, edge);
    case KeyShape.circle:
      canvas.drawCircle(pos, 3.6, body);
      canvas.drawCircle(pos, 3.6, edge);
    case KeyShape.diamond:
      const d = 4.0;
      final path = Path()
        ..moveTo(pos.dx, pos.dy - d)
        ..lineTo(pos.dx + d, pos.dy)
        ..lineTo(pos.dx, pos.dy + d)
        ..lineTo(pos.dx - d, pos.dy)
        ..close();
      canvas.drawPath(path, body);
      canvas.drawPath(path, edge);
  }
}
