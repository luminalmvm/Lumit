// Entering a Custom shader's inner graph (K-642, custom-shader.md §4.2, §8
// item 28): double-click the box, the Graph panel shows the inside with a
// breadcrumb back; Escape returns; the view you left is the view you return
// to, held in the session and never in the document.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/graph_panel.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';

import 'frb_test_support.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUpAll(initEngineForTests);

  group('Shader graph (frb)', () {
    ({LumitState state, LumitUiState uiState, LayerReference layer})
        withShader() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      comp.addSolidLayer();
      final layer = comp.getLayers().single;
      layer.addEffect(name: 'custom_shader');
      p.uiState.selectedLayer.value = layer;
      p.uiState.model.refresh();
      return (state: p.state, uiState: p.uiState, layer: layer);
    }

    Future<void> mount(WidgetTester tester, dynamic p) async {
      const size = Size(900, 600);
      tester.view.physicalSize = size;
      tester.view.devicePixelRatio = 1.0;
      addTearDown(tester.view.reset);
      await tester.pumpWidget(hostPanel(
        child: const GraphPanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: size,
      ));
      await tester.pump();
    }

    String effectKey(LayerReference layer) => graphNodeKey(layer
        .getGraph()
        .nodes
        .firstWhere((n) => n.matchName == 'custom_shader')
        .node);

    Future<void> enter(WidgetTester tester, LayerReference layer) async {
      final box = tester.getCenter(
          find.byKey(ValueKey<String>('graph-node-${effectKey(layer)}')));
      await tester.tapAt(box);
      await tester.pump(kDoubleTapMinTime);
      await tester.tapAt(box);
      await tester.pump();
    }

    testWidgets('entering a shader shows a breadcrumb and escape returns',
        (tester) async {
      final p = withShader();
      await mount(tester, p);

      await enter(tester, p.layer);
      expect(find.byKey(const ValueKey<String>('shader-breadcrumb')),
          findsOneWidget,
          reason: 'double-clicking the box opens the inside');
      // A shader that has never held a graph starts from a staged Result box.
      expect(find.byKey(const ValueKey<String>('shader-node-1')),
          findsOneWidget);
      // Nothing was committed by merely entering: the document has no graph.
      expect(
          p.layer
              .getEffects()
              .single
              .shaderGraph(),
          isNull,
          reason: 'entering is not an edit');

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pump();
      expect(find.byKey(const ValueKey<String>('shader-breadcrumb')),
          findsNothing,
          reason: 'Escape returns to the layer graph');
      expect(find.byKey(const ValueKey<String>('graph-toolbar')),
          findsOneWidget);

      // The breadcrumb's crumbs are the other way back.
      await enter(tester, p.layer);
      await tester
          .tap(find.byKey(const ValueKey<String>('shader-crumb-layer')));
      await tester.pump();
      expect(find.byKey(const ValueKey<String>('shader-breadcrumb')),
          findsNothing);
    });

    testWidgets('the view comes back on re-entry and is absent from the document',
        (tester) async {
      final p = withShader();
      await mount(tester, p);
      await enter(tester, p.layer);

      // Drag the canvas somewhere with the middle button (the pan).
      final canvas = tester
          .getCenter(find.byKey(const ValueKey<String>('shader-canvas')));
      final gesture = await tester.startGesture(canvas,
          kind: PointerDeviceKind.mouse, buttons: kMiddleMouseButton);
      await gesture.moveBy(const Offset(120, 60));
      await gesture.up();
      await tester.pump();

      final held =
          p.uiState.shaderGraphViews.values.single;
      expect(held.pan, isNot(Offset.zero), reason: 'the pan was remembered');

      await tester.sendKeyEvent(LogicalKeyboardKey.escape);
      await tester.pump();
      await enter(tester, p.layer);
      expect(p.uiState.shaderGraphViews.values.single.pan, held.pan,
          reason: 'the view you left is the view you return to');
      // And none of it is an edit: the document still holds no graph, so
      // nothing here can reach an op or an undo step.
      expect(p.layer.getEffects().single.shaderGraph(), isNull);
    });

    testWidgets('adding a box commits the graph as one undo step',
        (tester) async {
      final p = withShader();
      await mount(tester, p);
      await enter(tester, p.layer);

      await tester.sendKeyEvent(LogicalKeyboardKey.tab);
      await tester.pump();
      expect(find.byKey(const ValueKey<String>('shader-search')),
          findsOneWidget);
      await tester.tap(find.byKey(const ValueKey<String>('shader-add-uv')));
      await tester.pump();

      final json = p.layer.getEffects().single.shaderGraph();
      expect(json, isNotNull, reason: 'the first gesture writes the graph');
      expect(json, contains('"uv"'));

      p.state.project!.undo();
      p.uiState.model.refresh();
      await tester.pump();
      expect(p.layer.getEffects().single.shaderGraph(), isNull,
          reason: 'one gesture, one undo step');
    });
  });
}
