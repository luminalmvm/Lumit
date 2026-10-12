// The shape tools' preview: what a drag shows before it commits.
//
// The regression these guard is a plain one. The preview asked the *selected
// layer* to place every point, so with nothing selected — which is exactly the
// case that makes a shape layer, and most of the reason to pick up a shape tool
// — it drew nothing at all. You dragged, saw nothing, let go, and a shape
// appeared. The painter takes a coordinate space now rather than a layer, and
// there is always a space: the composition's when there is no layer.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_shape_layer.dart';
import 'package:lumit_flutter/panels/viewer_shapes.dart';
import 'package:lumit_flutter/state/tools.dart';

/// A canvas that remembers what it was asked to draw.
///
/// The question these tests ask is "did anything get drawn, and in what?" — not
/// "which pixels changed" — so recording the calls answers it exactly, with no
/// rasteriser and no golden file to keep up to date.
class _RecordingCanvas implements Canvas {
  final List<({Path path, Paint paint})> paths = [];
  final List<({Offset centre, double radius})> circles = [];

  @override
  void drawPath(Path path, Paint paint) => paths.add((path: path, paint: paint));

  @override
  void drawCircle(Offset c, double radius, Paint paint) =>
      circles.add((centre: c, radius: radius));

  @override
  void drawLine(Offset a, Offset b, Paint paint) {}

  @override
  void noSuchMethod(Invocation invocation) {}
}

/// The composition's own placement: the picture sits at 100,50 on screen and is
/// drawn at half size, so a comp pixel is half a screen pixel.
ShapeSpace compSpace() => ShapeSpace(
      toScreen: (x, y) => Offset(100 + x * 0.5, 50 + y * 0.5),
      ofScreen: (at) => ((at.dx - 100) / 0.5, (at.dy - 50) / 0.5),
    );

ShapePreviewPainter painter({
  required Offset? from,
  required Offset? to,
  Color fill = const Color(0xFF3366CC),
  Color? stroke,
  double strokeWidth = 0,
}) =>
    ShapePreviewPainter(
      tool: ToolMode.shapeRectangle,
      space: compSpace(),
      fill: fill,
      stroke: stroke,
      strokeWidth: strokeWidth,
      from: from,
      to: to,
      square: false,
      draft: const PathDraft(),
      penPointer: null,
      handleFrom: null,
      handleTo: null,
      closing: false,
      accent: const Color(0xFF00FF88),
    );

void main() {
  group('the shape preview', () {
    test('places the shape where the drag was, in composition pixels', () {
      final canvas = _RecordingCanvas();
      painter(from: const Offset(120, 70), to: const Offset(200, 150))
          .paint(canvas, const Size(400, 300));

      // The drag ran 120,70 → 200,150 on screen. The space above puts the
      // picture at 100,50 at half size, so the shape must come back to the
      // same screen rectangle it was dragged out in.
      final box = canvas.paths.first.path.getBounds();
      expect(box.left, closeTo(120, 1));
      expect(box.top, closeTo(70, 1));
      expect(box.right, closeTo(200, 1));
      expect(box.bottom, closeTo(150, 1));
    });
  });
}
