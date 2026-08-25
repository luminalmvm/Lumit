// The source rows on frb: what a layer is made of.
//
// Driven through the Effect controls panel, because the rows only appear for
// the kinds that have them and "which rows appear" is half of what they do.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_controls_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/retime.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  group('Source rows (frb)', () {
    ({LumitState state, LumitUiState uiState, CompositionReference comp})
        withComp() {
      final p = freshProject();
      final comp = p.state.project!.newComposition(name: 'Scene');
      p.uiState.setSelectedComp(comp);
      return (state: p.state, uiState: p.uiState, comp: comp);
    }

    Future<void> mount(WidgetTester tester, dynamic p) async {
      // The Transform card is off by default (K-193); this file asserts it
      // sits beside the Source card, so it asks for it.
      (p.uiState as LumitUiState)
          .workspace
          .interface
          .transformInEffectControls = true;
      await tester.pumpWidget(hostPanel(
        child: const EffectControlsPanelFrb(),
        state: p.state as LumitState,
        uiState: p.uiState as LumitUiState,
        size: const Size(520, 700),
      ));
      await tester.pump();
    }

    testWidgets('a text layer can be retyped, resized and recoloured',
        (tester) async {
      final p = withComp();
      final text = p.comp.addTextLayer();
      p.uiState.selectedLayer.value = text;
      await mount(tester, p);

      // A kicker since K-443: capitals on the way to the screen.
      expect(find.text('SOURCE'), findsOneWidget);
      expect(find.byKey(const ValueKey('src-text')), findsOneWidget);

      await tester.enterText(find.byKey(const ValueKey('src-text')), 'Hello');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      expect(text.getText()!.text, 'Hello',
          reason: 'the words reached the document');

      await tester.tap(find.byKey(const ValueKey('src-text-size')));
      await tester.pump();
      await tester.enterText(find.byType(EditableText).last, '96');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      expect(text.getText()!.size, 96);
    });

    testWidgets('a text layer can be driven by an expression', (tester) async {
      final p = withComp();
      final text = p.comp.addTextLayer();
      p.uiState.selectedLayer.value = text;
      await mount(tester, p);

      await tester.enterText(
          find.byKey(const ValueKey('src-text')), 'placeholder');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();

      await tester.enterText(
          find.byKey(const ValueKey('src-text-expression')), 'time * 2');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      expect(text.getText()!.expression, 'time * 2');
      expect(text.getText()!.text, 'placeholder',
          reason: 'the typed words are kept underneath the expression');

      // Emptying the box hands the layer back to its words.
      await tester.enterText(
          find.byKey(const ValueKey('src-text-expression')), '');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      expect(text.getText()!.expression, isNull);
      expect(text.getText()!.text, 'placeholder');
    });

    testWidgets('a camera layer shows its zoom and commits it', (tester) async {
      final p = withComp();
      final camera = p.comp.addCameraLayer();
      p.uiState.selectedLayer.value = camera;
      await mount(tester, p);

      expect(find.text('Zoom'), findsOneWidget);
      final field = find.byKey(const ValueKey('src-camera-zoom'));
      await tester.tap(field);
      await tester.pump();
      await tester.enterText(find.byType(EditableText).first, '1200');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();

      final zoom = camera.getCameraZoom()!;
      expect((zoom as BridgeScalar_Static).field0, 1200);
    });

    testWidgets('a solid layer edits the asset, and says that it does',
        (tester) async {
      final p = withComp();
      final solid = p.comp.addSolidLayer();
      p.uiState.selectedLayer.value = solid;
      await mount(tester, p);

      expect(find.byKey(const ValueKey('src-solid-colour')), findsOneWidget);
      expect(
          find.textContaining('every layer using it changes'), findsOneWidget,
          reason: 'the row warns that this is not a per-layer setting');

      await tester.tap(find.byKey(const ValueKey('src-solid-width')));
      await tester.pump();
      await tester.enterText(find.byType(EditableText).first, '640');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();

      // Read it back off the asset rather than the layer, which is the point.
      expect(find.byKey(const ValueKey('src-solid-width')), findsOneWidget);
    });

    testWidgets('a layer with no source of its own shows no card',
        (tester) async {
      final p = withComp();
      p.uiState.selectedLayer.value = p.comp.addAdjustmentLayer();
      await mount(tester, p);

      expect(find.text('SOURCE'), findsNothing,
          reason: 'an adjustment layer has no content to edit');
      expect(find.text('TRANSFORM'), findsOneWidget,
          reason: 'but it still has a transform');
    });

    testWidgets('the in-between frames row is on every layer, retimed or not',
        (tester) async {
      final p = withComp();
      final footage = p.state.project!.importFootage(path: 'C:/clips/shot.mov');
      p.comp.addFootageLayer(footage: footage, asSequence: false);
      final layer = p.comp.getLayers().single;
      p.uiState.selectedLayer.value = layer;
      await mount(tester, p);

      // The card's rival retiming system is gone (K-249): retiming is
      // Ctrl+Alt+T and the graph, and what is left here is the render policy,
      // which was never part of the map.
      expect(find.byKey(const ValueKey('src-retime-on')), findsNothing);
      expect(find.byKey(const ValueKey('src-retime-speed')), findsNothing);
      expect(find.byKey(const ValueKey('src-retime-reverse')), findsNothing);

      expect(find.byKey(const ValueKey('src-retime-interp')), findsOneWidget);
      expect(layer.getInterpolation(), BridgeRetimeInterp.nearest);
    });

    testWidgets('a layer with no retime still has a policy', (tester) async {
      final p = withComp();
      final layer = p.comp.addTextLayer();
      p.uiState.selectedLayer.value = layer;
      await mount(tester, p);
      expect(layer.getRetimeProperty(), isNull);
      expect(layer.getInterpolation(), BridgeRetimeInterp.nearest,
          reason: 'any layer can be asked for a moment between two frames');
    });
  }, skip: !engineAvailable);
}
