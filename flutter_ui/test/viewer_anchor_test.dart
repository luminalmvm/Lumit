// The Anchor point (pan behind) tool's two modifiers (K-220): the key points a
// held Ctrl snaps the pivot to, and the axis a held Shift locks the drag to.
//
// The compensation itself — the Position that cancels the jump — is
// `panBehindPosition`, ported and tested in viewer_layer_map's own suite; what
// is new here is which anchor a gesture asks for in the first place.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_anchor.dart';
import 'package:lumit_flutter/panels/viewer_gizmo.dart';
import 'package:lumit_flutter/panels/viewer_layer_map.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

void main() {
  /// A 200×100 layer at (300, 200), anchored on its middle, drawn 1:1.
  LayerBox box({double scale = 100, double rotation = 0}) => LayerBox(
        layer: LayerReference(
          internalprojectId: UuidValue.fromString(const Uuid().v4()),
          internalcompId: UuidValue.fromString(const Uuid().v4()),
          internallayerId: UuidValue.fromString(const Uuid().v4()),
        ),
        id: UuidValue.fromString(const Uuid().v4()),
        map: ViewerLayerMap.of(
          positionX: 300,
          positionY: 200,
          anchorX: 100,
          anchorY: 50,
          scaleXPercent: scale,
          scaleYPercent: scale,
          rotationDegrees: rotation,
          origin: Offset.zero,
          viewScale: 1,
        ),
        bounds: const Size(200, 100),
        draggable: true,
        scalable: true,
        rotationDegrees: rotation,
      );

  group('The key points a pivot snaps to', () {
    test('there are nine of them: corners, edge middles and the centre', () {
      final points = anchorKeyPoints(const Size(200, 100));
      expect(points.length, 9);
      expect(points, contains(Offset.zero));
      expect(points, contains(const Offset(200, 100)));
      expect(points, contains(const Offset(100, 50)));
      expect(points, contains(const Offset(200, 50)));
    });
  });

  group('Snapping the pivot', () {
    test('a pivot near a corner lands exactly on it', () {
      final snapped = snapAnchor(const Offset(6, 5), box());
      expect(snapped, Offset.zero);
    });

    test('a pivot in open ground is left where it is', () {
      const loose = Offset(60, 30);
      expect(snapAnchor(loose, box()), loose);
    });

    test('the distance is measured on screen, not in layer pixels', () {
      // At 10% the whole 200-pixel layer is 20 pixels wide, so a point 6 layer
      // pixels from the corner is well within the screen tolerance — where at
      // 100% it is on the edge of it and at 1000% it is far outside.
      expect(snapAnchor(const Offset(6, 5), box(scale: 10)), Offset.zero,
          reason: 'shrunk right down, everything is near everything');
      expect(snapAnchor(const Offset(6, 5), box(scale: 1000)),
          const Offset(6, 5),
          reason: 'blown right up, six layer pixels is half the screen');
    });

    test('it follows the layer round when the layer is turned', () {
      // The maths goes through the layer's own map either way, so a turned
      // layer's corners are still its corners.
      expect(snapAnchor(const Offset(4, 4), box(rotation: 37)), Offset.zero);
    });
  });

  group('The axis lock', () {
    test('a mostly-sideways drag loses its vertical', () {
      expect(constrainToAxis(const Offset(40, 6)), const Offset(40, 0));
    });

    test('a mostly-vertical drag loses its horizontal', () {
      expect(constrainToAxis(const Offset(-3, 25)), const Offset(0, 25));
    });

    test('an exactly diagonal drag picks the horizontal rather than dithering',
        () {
      expect(constrainToAxis(const Offset(10, 10)), const Offset(10, 0));
    });
  });
}
