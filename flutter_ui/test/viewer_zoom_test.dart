// The Viewer's zoom arithmetic: what a click, a wheel notch and a
// dragged box each mean in magnification and pan.
//
// The property that matters in all three is the same and is what these pin: the
// point you aimed at does not move. Everything else — the step size, the fit,
// the clamp — is arithmetic around that.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_zoom.dart';

void main() {
  const compSize = Size(1000, 500);
  const panel = Size(800, 400);
  const fitted = Rect.fromLTWH(0, 0, 800, 400);

  Offset after(ViewerZoom zoom, Offset screen) {
    final s1 = fitted.width / compSize.width;
    final u = (screen - fitted.topLeft) / s1;
    final topLeft = Offset(
          (panel.width - compSize.width * zoom.scale) / 2,
          (panel.height - compSize.height * zoom.scale) / 2,
        ) +
        zoom.pan;
    return topLeft + u * zoom.scale;
  }

  group('Zooming about a point', () {
    test('doubles the magnification and keeps the point under the pointer', () {
      const cursor = Offset(200, 300);
      final zoom = zoomAboutPoint(
        cursor: cursor,
        factor: 2,
        fitted: fitted,
        compSize: compSize,
        panel: panel,
      );
      expect(zoom.scale, closeTo(1.6, 1e-9));
      expect(after(zoom, cursor).dx, closeTo(cursor.dx, 1e-9));
      expect(after(zoom, cursor).dy, closeTo(cursor.dy, 1e-9));
    });
  });

  group('Zooming to a box', () {
    test('the box fills the panel and lands in its middle', () {
      const box = Rect.fromLTWH(300, 150, 200, 100);
      final zoom = zoomToBox(
        box: box,
        out: false,
        fitted: fitted,
        compSize: compSize,
        panel: panel,
      );
      expect(zoom.scale, closeTo(3.2, 1e-9));
      final centre = after(zoom, box.center);
      expect(centre.dx, closeTo(panel.width / 2, 1e-9));
      expect(centre.dy, closeTo(panel.height / 2, 1e-9));
    });

    test('a box of nothing does not divide by zero', () {
      final zoom = zoomToBox(
        box: const Rect.fromLTWH(400, 200, 0, 0),
        out: false,
        fitted: fitted,
        compSize: compSize,
        panel: panel,
      );
      expect(zoom.scale, maxViewerZoom,
          reason: 'clamped, not infinite — and certainly not NaN');
      expect(zoom.pan.dx.isFinite, isTrue);
      expect(zoom.pan.dy.isFinite, isTrue);
    });
  });
}
