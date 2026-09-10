// Layer ▸ Camera settings (docs/impl/camera.md §9).
//
// Two things are worth holding here. A preset is a focal length and the
// document holds a zoom, so picking one has to land the zoom the engine's own
// lens arithmetic gives - anything worked out a second time in Dart would drift
// from what the renderer draws. And the window writes when its button is
// pressed and at no other time: one settings op, one transform batch for the
// still numbers that moved, and nothing at all from Cancel.
//
// It lives beside the other engine-backed tests rather than in `test/` proper
// because it needs a real `LayerReference` to write through; there is nothing
// to substitute for one.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/camera_settings_dialog.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('the camera settings window', () {
    /// A fresh project, a comp, and a camera in it - with the settings and
    /// channels the window opens on.
    ({
      LayerReference layer,
      double width,
      Widget host,
    }) withCamera(({LumitState state, LumitUiState uiState}) p) {
      final comp = p.state.project!.newComposition(name: 'Scene');
      final layer = comp.addCameraLayer();
      final width = comp.getSettings().width.toDouble();
      return (
        layer: layer,
        width: width,
        host: Builder(
          builder: (context) => GestureDetector(
            key: const ValueKey('open'),
            behavior: HitTestBehavior.opaque,
            onTap: () => showCameraSettingsFrb(
              context: context,
              layer: layer,
              settings: layer.getCameraSettings()!,
              channels: layer.getTransform().camera!,
              compWidth: width,
            ),
            child: const SizedBox(width: 200, height: 40),
          ),
        ),
      );
    }

    testWidgets('picking a preset writes the zoom that focal length means',
        (tester) async {
      final p = freshProject();
      final camera = withCamera(p);

      await tester.pumpWidget(hostPanel(
          child: camera.host, state: p.state, uiState: p.uiState));
      await tester.tap(find.byKey(const ValueKey('open')));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('camera-preset')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('135 mm').last);
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('camera-apply')));
      await tester.pumpAndSettle();

      final film = camera.layer.getCameraSettings()!.filmSizeMm;
      final wanted = cameraZoomForFocal(
          focalMm: 135, filmMm: film, compW: camera.width);
      final zoom = camera.layer.getTransform().camera!.zoom;
      expect(stillValue(zoom), closeTo(wanted, 0.001),
          reason: 'the lens maths is the engine\'s, not a second copy of it');
    });

    testWidgets('the button commits one settings write and nothing else',
        (tester) async {
      final p = freshProject();
      final camera = withCamera(p);
      final before = camera.layer.getTransform();

      await tester.pumpWidget(hostPanel(
          child: camera.host, state: p.state, uiState: p.uiState));
      await tester.tap(find.byKey(const ValueKey('open')));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('camera-type')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Two-node').last);
      await tester.pumpAndSettle();
      await tester.tap(find.byKey(const ValueKey('camera-dof')));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('camera-apply')));
      await tester.pumpAndSettle();

      final after = camera.layer.getCameraSettings()!;
      expect(after.twoNode, isTrue);
      expect(after.depthOfField, isTrue);
      expect(camera.layer.getTransform(), before,
          reason: 'no number changed, so no transform op was written');

      // One op, so one step back: the whole of what the window did undoes at
      // once.
      p.state.project!.undo();
      expect(camera.layer.getCameraSettings()!.twoNode, isFalse);
      expect(camera.layer.getCameraSettings()!.depthOfField, isFalse);
    });

    testWidgets('Cancel writes nothing', (tester) async {
      final p = freshProject();
      final camera = withCamera(p);
      final settings = camera.layer.getCameraSettings()!;
      final transform = camera.layer.getTransform();

      await tester.pumpWidget(hostPanel(
          child: camera.host, state: p.state, uiState: p.uiState));
      await tester.tap(find.byKey(const ValueKey('open')));
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('camera-preset')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('15 mm').last);
      await tester.pumpAndSettle();

      await tester.tap(find.byKey(const ValueKey('camera-cancel')));
      await tester.pumpAndSettle();

      expect(camera.layer.getCameraSettings(), settings);
      expect(camera.layer.getTransform(), transform);
    });

    testWidgets('an animated channel says so and is left alone', (tester) async {
      final p = freshProject();
      final camera = withCamera(p);
      camera.layer.setTransform(
        prop: BridgeTransformProp.zoom,
        value: const BridgeScalar.keyframed([
          BridgeKeyframe(
            time: BridgeRational(num: 0, den: 1),
            value: 900,
            interpIn: BridgeSideInterp.linear(),
            interpOut: BridgeSideInterp.linear(),
          ),
        ]),
      );
      final keyed = camera.layer.getTransform().camera!.zoom;

      await tester.pumpWidget(hostPanel(
          child: camera.host, state: p.state, uiState: p.uiState));
      await tester.tap(find.byKey(const ValueKey('open')));
      await tester.pumpAndSettle();

      expect(find.byKey(const ValueKey('camera-zoom')), findsNothing);
      expect(find.byKey(const ValueKey('camera-zoom-animated')), findsOneWidget,
          reason: 'a still number typed over a curve would delete it');

      await tester.tap(find.byKey(const ValueKey('camera-apply')));
      await tester.pumpAndSettle();

      expect(camera.layer.getTransform().camera!.zoom, keyed);
    });
  }, skip: !engineAvailable);
}
