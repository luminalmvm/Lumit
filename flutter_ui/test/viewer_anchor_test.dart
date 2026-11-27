
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/viewer_anchor.dart';
import 'package:lumit_flutter/panels/viewer_gizmo.dart';
import 'package:lumit_flutter/panels/viewer_layer_map.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

void main() {
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

  group('Snapping the pivot', () {
    test('a pivot near a corner lands exactly on it', () {
      final snapped = snapAnchor(const Offset(6, 5), box());
      expect(snapped, Offset.zero);
    });

    test('the distance is measured on screen, not in layer pixels', () {
      expect(snapAnchor(const Offset(6, 5), box(scale: 10)), Offset.zero,
          reason: 'shrunk right down, everything is near everything');
      expect(snapAnchor(const Offset(6, 5), box(scale: 1000)),
          const Offset(6, 5),
          reason: 'blown right up, six layer pixels is half the screen');
    });
  });

  group('The axis lock', () {
    test('a mostly-sideways drag loses its vertical', () {
      expect(constrainToAxis(const Offset(40, 6)), const Offset(40, 0));
    });
  });
}
