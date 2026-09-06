// Entering a Custom shader's inner graph (custom-shader.md §4.2, §8
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
import 'package:lumit_flutter/state/dock.dart';
import 'package:lumit_flutter/theme/theme.dart';

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

    /// **Ctrl+Space adds inside a shader** (the owner's "can't add
    /// options in the custom shader view"): with the inner graph the panel's
    /// face, the console lists the shader vocabulary and picking a row drops
    /// that box — committed as one undo step.
    testWidgets('the console adds a box, committed as one undo step',
        (tester) async {
      final p = withShader();
      await mount(tester, p);
      await enter(tester, p.layer);

      p.uiState.activePanel.value = Panel.graph;
      expect(p.uiState.consoleClaim!(), isTrue,
          reason: 'the inner graph claims Ctrl+Space while it is the face');
      await tester.pump();
      expect(find.byKey(const ValueKey<String>('fx-console-bar')),
          findsOneWidget);
      // The Parameter box is in the vocabulary — the row the fix is for.
      expect(find.byKey(const ValueKey<String>('fx-console-item-Parameter')),
          findsOneWidget);

      await tester
          .tap(find.byKey(const ValueKey<String>('fx-console-item-UV')));
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

    /// **Scrolling the console never zooms the inner graph** (owner item 12
    /// — the leak's original home). The old popover sat *inside* the canvas's
    /// pointer listener, so a wheel over its list also reached the zoom; the
    /// console floats in the overlay, which the wheel cannot pass.
    testWidgets('a wheel over the console leaves the inner zoom alone',
        (tester) async {
      final p = withShader();
      await mount(tester, p);
      await enter(tester, p.layer);

      final before =
          tester.getRect(find.byKey(const ValueKey<String>('shader-node-1')));

      p.uiState.activePanel.value = Panel.graph;
      expect(p.uiState.consoleClaim!(), isTrue);
      await tester.pump();

      final list = tester.getCenter(
          find.byKey(const ValueKey<String>('fx-console-item-Picture')));
      await tester.sendEventToBinding(
          PointerScrollEvent(position: list, scrollDelta: const Offset(0, 40)));
      await tester.pump();

      expect(
          tester.getRect(find.byKey(const ValueKey<String>('shader-node-1'))),
          before,
          reason: 'no zoom moved the box: the scroll was the console\'s');
    });

    /// **The inner graph wears the shader tint too**: every box in
    /// here is shader vocabulary, so every header carries the same wash the
    /// Custom shader box wears outside — one colour, one meaning.
    testWidgets('an inner box\'s header wears the shader tint',
        (tester) async {
      final p = withShader();
      await mount(tester, p);
      await enter(tester, p.layer);

      final theme = LumitTheme.forScheme(LumitColorScheme.dark, ThemeShape.sharp);
      final colours = [
        for (final c in tester.widgetList<Container>(find.descendant(
            of: find.byKey(const ValueKey<String>('shader-node-1')),
            matching: find.byType(Container))))
          if (c.decoration case final BoxDecoration d?) d.color,
      ];
      expect(colours, contains(graphShaderHeader(theme)));
    });

    /// A wire let go over empty canvas opens the same console, and the box
    /// lands where the wire was dropped — the road that used to raise the
    /// inner graph's own popover, which is gone now.
    testWidgets('a wire dropped on empty canvas opens the console',
        (tester) async {
      final p = withShader();
      await mount(tester, p);
      await enter(tester, p.layer);

      // The Result box's one input socket: drag a wire out of it backwards
      // onto empty ground. Grabbing an unwired input starts a fresh wire.
      final socket = tester.getCenter(find.byKey(
          const ValueKey<String>('shader-socket-1-in-colour')));
      await tester.dragFrom(socket, const Offset(-160, 120));
      await tester.pump();

      expect(find.byKey(const ValueKey<String>('fx-console-bar')),
          findsOneWidget,
          reason: 'the drop summons the console, not a second search');
      expect(find.byKey(const ValueKey<String>('fx-console-item-Result')),
          findsNothing,
          reason: 'one Result box is the law, so the row is withheld');
    });
  });
}
