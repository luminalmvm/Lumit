// What a Camera row and a Light row are made of (docs/impl/camera.md §1).
//
// The rows a layer shows come out of `transformGroups`, and the two cells that
// differ on these kinds come out of two predicates beside it - so the whole
// answer is arithmetic and is checked here rather than by clicking in a widget
// tree, exactly as timeline_rows_test.dart checks the fold-out's own rules.

import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/layer_fold_frb.dart';
import 'package:lumit_flutter/panels/transform_rows_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart' show BridgeScalar;
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb/frb_test_support.dart';

void main() {
  const modes = BridgeAxisModes(
    anchor: BridgeAxisMode.combined,
    position: BridgeAxisMode.combined,
    scale: BridgeAxisMode.combined,
  );

  List<TransformGroup> rowsFor(BridgeLayerKind kind, {bool twoNode = false}) =>
      transformGroups(
          threeD: false, modes: modes, kind: kind, twoNode: twoNode);

  Set<BridgeTransformProp> propsOf(List<TransformGroup> groups) => {
        for (final group in groups)
          for (final axis in group.axes) axis.prop,
      };

  group('A camera shows what a viewpoint has', () {
    test('no anchor, no scale and no opacity', () {
      final props = propsOf(rowsFor(BridgeLayerKind.camera));
      expect(
          props,
          isNot(anyOf(
            contains(BridgeTransformProp.anchorX),
            contains(BridgeTransformProp.anchorY),
            contains(BridgeTransformProp.scaleX),
            contains(BridgeTransformProp.scaleY),
            contains(BridgeTransformProp.opacity),
          )),
          reason: 'they mean nothing on something that draws no pixels');
    });

    test('its place in three dimensions whatever the 3D switch says', () {
      final props = propsOf(rowsFor(BridgeLayerKind.camera));
      expect(
          props,
          containsAll([
            BridgeTransformProp.positionX,
            BridgeTransformProp.positionY,
            BridgeTransformProp.positionZ,
            BridgeTransformProp.rotationX,
            BridgeTransformProp.rotationY,
            BridgeTransformProp.rotation,
          ]));
    });

    test('the four camera options, and only those, are flagged as such', () {
      final options = [
        for (final group in rowsFor(BridgeLayerKind.camera))
          if (group.cameraOption)
            for (final axis in group.axes) axis.prop,
      ];
      expect(options, [
        BridgeTransformProp.zoom,
        BridgeTransformProp.focusDistance,
        BridgeTransformProp.aperture,
        BridgeTransformProp.blurLevel,
      ]);
    });

    test('the blur level is a percentage, and stops at a hundred', () {
      final blur = rowsFor(BridgeLayerKind.camera)
          .firstWhere((g) => g.axes.first.prop == BridgeTransformProp.blurLevel)
          .axes
          .single;
      expect(blur.suffix, '%');
      expect(blur.min, 0);
      expect(blur.max, 100);
    });

    test('a point of interest only where there is one to aim', () {
      expect(propsOf(rowsFor(BridgeLayerKind.camera)),
          isNot(contains(BridgeTransformProp.poiX)),
          reason: 'a one-node camera aims by its own rotation');
      expect(
          propsOf(rowsFor(BridgeLayerKind.camera, twoNode: true)),
          containsAll([
            BridgeTransformProp.poiX,
            BridgeTransformProp.poiY,
            BridgeTransformProp.poiZ,
          ]));
    });

    test('and it leads the list, above where the camera stands', () {
      final rows = rowsFor(BridgeLayerKind.camera, twoNode: true);
      expect(rows.first.axes.first.prop, BridgeTransformProp.poiX);
    });
  });

  group('A light shows the same placement and nothing else', () {
    test('no anchor, scale, opacity or camera options', () {
      final rows = rowsFor(BridgeLayerKind.light);
      expect(
          propsOf(rows),
          isNot(anyOf(
            contains(BridgeTransformProp.anchorX),
            contains(BridgeTransformProp.scaleX),
            contains(BridgeTransformProp.opacity),
            contains(BridgeTransformProp.zoom),
            contains(BridgeTransformProp.poiX),
          )));
      expect(rows.any((g) => g.cameraOption), isFalse);
    });

    test('its place and its three turns', () {
      expect(propsOf(rowsFor(BridgeLayerKind.light)), {
        BridgeTransformProp.positionX,
        BridgeTransformProp.positionY,
        BridgeTransformProp.positionZ,
        BridgeTransformProp.rotationX,
        BridgeTransformProp.rotationY,
        BridgeTransformProp.rotation,
      });
    });
  });

  group('Every other kind is left exactly as it was', () {
    test('a 2D footage layer still shows its eleven, minus the 3D three', () {
      expect(propsOf(rowsFor(BridgeLayerKind.footage)), {
        BridgeTransformProp.anchorX,
        BridgeTransformProp.anchorY,
        BridgeTransformProp.positionX,
        BridgeTransformProp.positionY,
        BridgeTransformProp.scaleX,
        BridgeTransformProp.scaleY,
        BridgeTransformProp.rotation,
        BridgeTransformProp.opacity,
      });
    });
  });

  group('The two cells a camera row draws differently', () {
    test('the eye is on a camera, picture or no picture', () {
      expect(
          hasVisibilitySwitch(BridgeLayerKind.camera, hasPicture: false), isTrue,
          reason: 'it is what makes the camera the active one');
      expect(hasVisibilitySwitch(BridgeLayerKind.audio, hasPicture: false),
          isFalse);
      expect(hasVisibilitySwitch(BridgeLayerKind.footage, hasPicture: true),
          isTrue);
    });

    test('the 3D cell is blank on a camera and on a light', () {
      expect(hasThreeDSwitch(BridgeLayerKind.camera), isFalse);
      expect(hasThreeDSwitch(BridgeLayerKind.light), isFalse);
      expect(hasThreeDSwitch(BridgeLayerKind.footage), isTrue);
      expect(hasThreeDSwitch(BridgeLayerKind.nullLayer), isTrue,
          reason: 'a null is placed in three dimensions by its own switch');
    });
  });

  group('A transform with no camera channels reads them as a still zero', () {
    BridgeScalar st(double v) => BridgeScalar.static_(v);
    final plain = BridgeTransform(
      anchorX: st(0),
      anchorY: st(0),
      positionX: st(0),
      positionY: st(0),
      positionZ: st(0),
      scaleX: st(100),
      scaleY: st(100),
      rotation: st(0),
      rotationX: st(0),
      rotationY: st(0),
      opacity: st(100),
    );

    test('reading one answers zero rather than throwing', () {
      expect(read(plain, BridgeTransformProp.zoom), const BridgeScalar.static_(0));
      expect(read(plain, BridgeTransformProp.poiX), const BridgeScalar.static_(0));
    });

    test('writing one leaves the transform without channels', () {
      expect(write(plain, BridgeTransformProp.zoom, 1400).camera, isNull);
    });

    test('and a camera keeps the other six when one is written', () {
      final camera = BridgeTransform(
        anchorX: st(0),
        anchorY: st(0),
        positionX: st(0),
        positionY: st(0),
        positionZ: st(0),
        scaleX: st(100),
        scaleY: st(100),
        rotation: st(0),
        rotationX: st(0),
        rotationY: st(0),
        opacity: st(100),
        camera: BridgeCameraChannels(
          poiX: st(1),
          poiY: st(2),
          poiZ: st(3),
          zoom: st(1000),
          focusDistance: st(1000),
          aperture: st(25),
          blurLevel: st(100),
        ),
      );
      final written = write(camera, BridgeTransformProp.zoom, 1400);
      expect(written.camera!.zoom, st(1400));
      expect(written.camera!.poiY, st(2));
      expect(written.camera!.blurLevel, st(100));
    });
  });

  // The fold-out itself needs a real layer to read, so this half runs against
  // the engine - there is nothing to substitute for a `LayerReference`.
  group('A camera\'s fold-out heads its lens numbers', () {
    setUpAll(initEngineForTests);

    ({CompositionReference comp, LayerReference camera}) withCamera() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (comp: comp, camera: comp.addCameraLayer());
    }

    List<LayerFoldRow> foldOf(CompositionReference comp, Set<String> open) =>
        layerFoldRows(
            entry: comp.getModel().layers.single, open: open, hasAudio: false);

    testWidgets('the Camera options heading appears with Transform open',
        (tester) async {
      final c = withCamera();
      final id = c.camera.internallayerId.toString();
      final rows = foldOf(c.comp, {transformPath(id)});

      expect(rows.whereType<FoldGroupRow>().map((g) => g.path),
          contains(cameraOptionsPath(id)));
      expect(
          rows.whereType<FoldTransformRow>().map((r) => r.group.label),
          isNot(anyOf(contains('Anchor point'), contains('Scale'),
              contains('Opacity'))));
      expect(rows.whereType<FoldTransformRow>().any((r) => r.group.cameraOption),
          isFalse,
          reason: 'the four are behind their own twirl until it is opened');
    });

    testWidgets('and its four rows come out from under it', (tester) async {
      final c = withCamera();
      final id = c.camera.internallayerId.toString();
      final rows =
          foldOf(c.comp, {transformPath(id), cameraOptionsPath(id)});

      expect(
          [
            for (final row in rows.whereType<FoldTransformRow>())
              if (row.group.cameraOption) row.group.label,
          ],
          ['Zoom', 'Focus distance', 'Aperture', 'Blur level']);
      expect(
          rows
              .whereType<FoldTransformRow>()
              .where((r) => r.group.cameraOption)
              .every((r) => r.depth == 3),
          isTrue,
          reason: 'they sit under the heading, not beside it');
    });

    testWidgets('a reveal on one of them opens that row alone', (tester) async {
      final c = withCamera();
      final id = c.camera.internallayerId.toString();
      final zoom = transformGroups(
        threeD: false,
        modes: c.comp.getModel().layers.single.info.axisModes,
        kind: BridgeLayerKind.camera,
      ).firstWhere((g) => g.axes.first.prop == BridgeTransformProp.zoom);
      final rows = foldOf(c.comp, {transformGroupPath(id, zoom)});

      expect(rows.whereType<FoldTransformRow>().map((r) => r.group.label),
          ['Zoom']);
    });
  }, skip: !engineAvailable);
}
