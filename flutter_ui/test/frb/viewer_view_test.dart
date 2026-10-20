// The Viewer's 3D views on frb, against the real engine (docs/impl/camera.md
// §6).
//
// Engine-backed because there is no other kind: a view *is* a pose the engine
// builds for the comp's size, choosing one sends it over the bridge, and the
// wireframes drawn in it are gathered there too. None of that has a
// stand-in.
//
// What is asserted is the panel's half of it: the picker lists the views and
// says which one is in force, and choosing one puts a pose on that view of the
// comp and not on the view beside it.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/icons/lumit_icon.dart' as glyph;
import 'package:lumit_flutter/icons/lumit_icons.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/state/viewer_view.dart';
import 'package:lumit_flutter/state/viewer_views.dart';

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
      // Wide enough for a two-up whose bars both keep their pickers.
      tester.view.physicalSize = const Size(1600, 500);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const ViewerPanelFrb(),
        state: p.state,
        uiState: p.uiState,
        size: const Size(1600, 500),
      ));
      await tester.pump();
    }

    /// Open the first view's picker, scrolling it into reach first, should
    /// its strip be narrower than the bar wants.
    Future<void> openPicker(WidgetTester tester) async {
      final button = find.byKey(const ValueKey('viewer-view')).first;
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

    testWidgets('choosing a view puts that view\'s pose on its own pane only',
        (tester) async {
      final p = withScene();
      await mount(tester, p);
      // Split into a two-up of the one comp, picked on the first pane's bar.
      final pane = Panel.viewer.pane();
      p.uiState.views.setLayout(pane, ViewLayout.twoAcross);
      await tester.pump();
      final [first, second] = p.uiState.views.forPane(pane);
      expect(second.compId, p.comp.internalid.toString());
      expect(p.uiState.viewPoseOf(first.engineId), isNull,
          reason: 'the active camera is not a pose of the panel\'s');

      await pickView(tester, ViewerView.front);
      expect(p.uiState.viewOf(first.engineId), ViewerView.front);
      final front = p.uiState.viewPoseOf(first.engineId);
      expect(front, isNotNull,
          reason: 'the engine answered with the Front view\'s pose');
      expect(p.uiState.viewPoseOf(second.engineId), isNull,
          reason: 'the pane beside it still looks through the comp camera');
      // §6: the fixed views look at the comp centre from 100 · width away,
      // with a zoom to match, which is orthographic to the pixel.
      final size = p.comp.getSize();
      expect(front!.zoom, closeTo(100.0 * size.width, 1e-6));
      expect(front.z, closeTo(-100.0 * size.width, 1e-6));
      expect(front.rotationY, closeTo(0, 1e-9));

      // Back looks the other way, and is its own stored pose.
      await pickView(tester, ViewerView.back);
      expect(p.uiState.viewPoseOf(first.engineId)!.rotationY,
          closeTo(180, 1e-6));

      await openPicker(tester);
      expect(
        find.descendant(
          of: find.byKey(const ValueKey('viewer-view-back')),
          matching: _tick,
        ),
        findsOneWidget,
      );
    });

  }, skip: !engineAvailable);
}
