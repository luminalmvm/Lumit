// The Viewer's 3D views on frb, against the real engine (docs/impl/camera.md
// §6).
//
// Engine-backed because there is no other kind: a view *is* a pose the engine
// builds for the comp's size, choosing one sends it over the bridge, and the
// wireframes drawn in it are gathered there too. None of that has a
// stand-in.
//
// What is asserted is the panel's half of it: the picker lists the views and
// says which one is in force, choosing one puts a pose on the comp, and the
// wireframes are drawn in a view and not through the composition's own camera.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/icons/lumit_icon.dart' as glyph;
import 'package:lumit_flutter/icons/lumit_icons.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart' show BridgeCameraPose;
import 'package:lumit_flutter/state/viewer_view.dart';

import 'frb_test_support.dart';

/// A ticked menu row's mark: the set's checkmark.
final Finder _tick = find.byWidgetPredicate(
    (w) => w is glyph.LumitIcon && w.glyph == LumitIcons.tick);

void main() {
  setUpAll(initEngineForTests);

  group('The Viewer\'s 3D views (frb)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withScene() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      // A camera and a light, so the wireframes have something to draw beyond
      // the layer rectangles.
      comp.addCameraLayer();
      comp.addLightLayer(kind: 0);
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    Future<void> mount(
      WidgetTester tester,
      ({LumitState state, LumitUiState uiState, CompositionReference comp}) p,
    ) async {
      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(700, 500),
      ));
      await tester.pump();
    }

    /// Open the bar's view picker, scrolling it into reach first: this Viewer
    /// is narrower than the bar wants, so the strip scrolls.
    Future<void> openPicker(WidgetTester tester) async {
      final button = find.byKey(const ValueKey('viewer-view'));
      await tester.ensureVisible(button);
      await tester.pump();
      await tester.tap(button);
      await tester.pumpAndSettle();
    }

    Future<void> pickView(WidgetTester tester, ViewerView view) async {
      await openPicker(tester);
      await tester.tap(find.byKey(ValueKey<String>('viewer-view-${view.name}')));
      await tester.pumpAndSettle();
    }

    testWidgets('the picker lists every view and ticks the one in force',
        (tester) async {
      final p = withScene();
      await mount(tester, p);
      await openPicker(tester);

      for (final view in ViewerView.values) {
        expect(find.byKey(ValueKey<String>('viewer-view-${view.name}')),
            findsOneWidget,
            reason: '${view.name} must be offered');
      }
      expect(_tick, findsOneWidget, reason: 'exactly one view is in force');
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('viewer-view-activeCamera')),
          matching: _tick,
        ),
        findsOneWidget,
        reason: 'a comp opens on its own camera',
      );
    });

    testWidgets('choosing a view puts that view\'s pose on the composition',
        (tester) async {
      final p = withScene();
      await mount(tester, p);
      expect(p.uiState.viewerViewPose, isNull,
          reason: 'the active camera is not a pose of the panel\'s');

      await pickView(tester, ViewerView.front);
      expect(p.uiState.viewerView, ViewerView.front);
      final front = p.uiState.viewerViewPose;
      expect(front, isNotNull,
          reason: 'the engine answered with the Front view\'s pose');
      // §6: the fixed views look at the comp centre from 100 · width away,
      // with a zoom to match, which is orthographic to the pixel.
      final size = p.comp.getSize();
      expect(front!.zoom, closeTo(100.0 * size.width, 1e-6));
      expect(front.z, closeTo(-100.0 * size.width, 1e-6));
      expect(front.rotationY, closeTo(0, 1e-9));

      // Back looks the other way, and is its own stored pose.
      await pickView(tester, ViewerView.back);
      expect(p.uiState.viewerViewPose!.rotationY, closeTo(180, 1e-6));

      await openPicker(tester);
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('viewer-view-back')),
          matching: _tick,
        ),
        findsOneWidget,
      );
    });

    testWidgets('the wireframes are drawn in a view and not through the camera',
        (tester) async {
      final p = withScene();
      await mount(tester, p);
      final wireframes = find.byKey(const ValueKey('viewer-wireframes'));
      expect(wireframes, findsNothing,
          reason: 'the picture itself is what the active camera sees');

      await pickView(tester, ViewerView.top);
      expect(wireframes, findsOneWidget,
          reason: 'a view needs the marks that say where things are');

      await pickView(tester, ViewerView.activeCamera);
      expect(wireframes, findsNothing);
    });

    testWidgets('only a custom view keeps what the camera tools do to it',
        (tester) async {
      final p = withScene();
      await mount(tester, p);

      // A fixed view is not moved: a Front view dragged off square would not
      // be a front view, and the tools say so rather than bending it.
      await pickView(tester, ViewerView.left);
      expect(ViewerView.left.movable, isFalse);
      final square = p.uiState.viewerViewPose;
      p.uiState.setViewerViewPose(BridgeCameraPose(
        zoom: 1,
        x: 2,
        y: 3,
        z: 4,
        rotationX: 5,
        rotationY: 6,
        rotationZ: 7,
      ));
      expect(p.uiState.viewerViewPose, square);

      await pickView(tester, ViewerView.custom1);
      final start = p.uiState.viewerViewPose;
      expect(start, isNotNull, reason: 'it starts at the three-quarter view');
      p.uiState.setViewerViewPose(BridgeCameraPose(
        zoom: start!.zoom,
        x: start.x + 100,
        y: start.y,
        z: start.z,
        rotationX: start.rotationX,
        rotationY: start.rotationY,
        rotationZ: start.rotationZ,
      ));
      await tester.pumpAndSettle();
      expect(p.uiState.viewerViewPose!.x, closeTo(start.x + 100, 1e-9));

      // And it is still there when the view is left and come back to.
      await pickView(tester, ViewerView.top);
      await pickView(tester, ViewerView.custom1);
      expect(p.uiState.viewerViewPose!.x, closeTo(start.x + 100, 1e-9));
    });
  }, skip: !engineAvailable);
}
