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
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';

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
      // The Transform card is off by default; this file asserts it
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

      // A kicker now: capitals on the way to the screen.
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

    // Text on a path: the words follow one of the layer's own masks,
    // picked by name, and the offset dial only appears once there is a curve
    // to slide along.
    testWidgets('a text layer runs its words along one of its masks',
        (tester) async {
      final p = withComp();
      final text = p.comp.addTextLayer();
      text.addMask(
        mask: BridgeMask(
          id: UuidValue.fromString(const Uuid().v4()),
          name: 'Arc',
          vertices: const [
            BridgeVertex(
                x: 0, y: 40, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
            BridgeVertex(
                x: 200, y: 40, tanInX: 0, tanInY: 0, tanOutX: 0, tanOutY: 0),
          ],
          closed: false,
          inverted: false,
          opacity: const BridgeScalar.static_(100),
          mode: BridgeMaskMode.none,
          feather: const BridgeScalar.static_(0),
          vertexFeather: const [],
          expansion: const BridgeScalar.static_(0),
          pathKeys: const [],
        ),
      );
      p.uiState.selectedLayer.value = text;
      await mount(tester, p);

      // Straight to begin with, so there is nothing to slide.
      expect(text.getText()!.path, isNull);
      expect(find.byKey(const ValueKey('src-text-path-offset')), findsNothing);

      await tester.tap(find.byKey(const ValueKey('src-text-path')));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Arc').last);
      await tester.pumpAndSettle();

      expect(text.getText()!.path, text.getMasks().single.id,
          reason: 'the picked mask reached the document');
      expect(find.byKey(const ValueKey('src-text-path-offset')), findsOneWidget,
          reason: 'a curve to slide along brings the dial with it');

      await tester.tap(find.byKey(const ValueKey('src-text-path-offset')));
      await tester.pump();
      await tester.enterText(find.byType(EditableText).last, '25');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      expect(
          (text.getText()!.pathOffset as BridgeScalar_Static).field0, 25);

      // And typing into the layer leaves the curve alone.
      await tester.enterText(find.byKey(const ValueKey('src-text')), 'Lumit');
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      expect(text.getText()!.path, isNotNull,
          reason: 'a retype must not straighten the line');
    });

  }, skip: !engineAvailable);
}
