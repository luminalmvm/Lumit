// The Text and Paragraph panels on frb: what they write to a selected text
// layer, and what they keep for new text when none is selected.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/paragraph_panel_frb.dart';
import 'package:lumit_flutter/panels/text_panel_frb.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/text_documents.dart';

import 'frb_test_support.dart';

void main() {
  setUpAll(initEngineForTests);

  ({LumitState state, LumitUiState uiState, CompositionReference comp})
      withComp() {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Scene');
    p.uiState.setSelectedComp(comp);
    return (state: p.state, uiState: p.uiState, comp: comp);
  }

  Future<void> mount(WidgetTester tester, dynamic p, Widget panel) async {
    (p.uiState as LumitUiState).model.refresh();
    await tester.pumpWidget(hostPanel(
      child: panel,
      state: p.state as LumitState,
      uiState: p.uiState as LumitUiState,
      size: const Size(320, 700),
    ));
    await tester.pump();
  }

  /// Type [value] into the value well keyed [name].
  Future<void> typeInto(WidgetTester tester, String name, String value) async {
    await tester.tap(find.byKey(ValueKey<String>(name)));
    await tester.pump();
    await tester.enterText(find.byType(EditableText).last, value);
    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pump();
  }

  double anchorX(LayerReference layer) =>
      (layer.getTransform().anchorX as BridgeScalar_Static).field0;

  test('the plain style held in Dart is the engine\'s own', () {
    expect(plainTextStyle, defaultTextStyle());
    expect(plainParagraphStyle, defaultParagraphStyle());
  });

  group('Text panel (frb)', () {
    testWidgets('a selected text layer is restyled, one undo step a change',
        (tester) async {
      final p = withComp();
      final text = p.comp.addTextLayer();
      p.uiState.selectedLayer.value = text;
      await mount(tester, p, const TextPanelFrb());

      expect(find.text('Text'), findsOneWidget,
          reason: 'the heading names the layer being edited');

      await typeInto(tester, 'text-size', '96');
      expect(text.getText()!.size, 96);

      await typeInto(tester, 'text-tracking', '50');
      expect(text.getText()!.style.tracking, 50);

      await tester.tap(find.byKey(const ValueKey('text-kerning')));
      await tester.pump();
      await tester.tap(find.text('Metrics').last);
      await tester.pump();
      expect(text.getText()!.style.kerning, BridgeKerning.metrics);

      await tester.tap(find.byKey(const ValueKey('text-stroke-on')));
      await tester.pump();
      expect(text.getText()!.style.strokeOn, isTrue);

      p.state.project!.undo();
      expect(text.getText()!.style.strokeOn, isFalse);
      expect(text.getText()!.style.kerning, BridgeKerning.metrics,
          reason: 'one undo took back one change');
    });

    testWidgets('More shows the rest, and the capitals are one choice',
        (tester) async {
      final p = withComp();
      final text = p.comp.addTextLayer();
      p.uiState.selectedLayer.value = text;
      await mount(tester, p, const TextPanelFrb());

      expect(find.byKey(const ValueKey('text-all-caps')), findsNothing);
      await tester.tap(find.byKey(const ValueKey('text-more')));
      await tester.pump();

      await tester.tap(find.byKey(const ValueKey('text-all-caps')));
      await tester.pump();
      expect(text.getText()!.style.caps, BridgeCaps.all);
      await tester.tap(find.byKey(const ValueKey('text-small-caps')));
      await tester.pump();
      expect(text.getText()!.style.caps, BridgeCaps.small);
      await tester.tap(find.byKey(const ValueKey('text-small-caps')));
      await tester.pump();
      expect(text.getText()!.style.caps, BridgeCaps.normal);

      await typeInto(tester, 'text-scale-x', '50');
      expect(text.getText()!.style.scaleX, 50);

      // Auto leading shows the number it comes to, and unticking keeps it.
      expect(text.getText()!.style.leading, isNull);
      await tester.tap(find.byKey(const ValueKey('text-leading-auto')));
      await tester.pump();
      expect(text.getText()!.style.leading, closeTo(72 * 1.2, 1e-9));
    });

    testWidgets('every selected text layer takes the change, as one step',
        (tester) async {
      final p = withComp();
      final first = p.comp.addTextLayer();
      final second = p.comp.addTextLayer();
      p.uiState.setSelection([first, second]);
      await mount(tester, p, const TextPanelFrb());

      await typeInto(tester, 'text-tracking', '120');
      expect(first.getText()!.style.tracking, 120);
      expect(second.getText()!.style.tracking, 120);

      p.state.project!.undo();
      expect(first.getText()!.style.tracking, 0);
      expect(second.getText()!.style.tracking, 0);
    });

    testWidgets('with no text layer selected it sets what new text is made in',
        (tester) async {
      final p = withComp();
      await mount(tester, p, const TextPanelFrb());

      expect(find.text('New text'), findsOneWidget);
      await typeInto(tester, 'text-size', '48');
      await typeInto(tester, 'text-tracking', '25');
      expect(p.uiState.tools.textSize, 48);
      expect(p.uiState.tools.textStyle.tracking, 25);
    });

    testWidgets('a font this machine lacks is named, not hidden',
        (tester) async {
      final p = withComp();
      final text = p.comp.addTextLayer();
      final document = text.getText()!;
      text.setText(
        document: document.copyWith(
          style: document.style.copyWith(family: 'No Such Family 6f1c'),
        ),
      );
      p.uiState.selectedLayer.value = text;
      await mount(tester, p, const TextPanelFrb());
      // The font list arrives a moment after the panel does.
      await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 300)));
      await tester.pump();
      expect(find.text('No Such Family 6f1c (not installed)'), findsOneWidget);
    });
  });

  group('Paragraph panel (frb)', () {
    testWidgets('alignment and spacing reach the layer', (tester) async {
      final p = withComp();
      final text = p.comp.addTextLayer();
      p.uiState.selectedLayer.value = text;
      await mount(tester, p, const ParagraphPanelFrb());

      await tester.tap(find.byKey(const ValueKey('paragraph-align-centre')));
      await tester.pump();
      expect(text.getText()!.paragraph.align, BridgeTextAlign.centre);

      await typeInto(tester, 'paragraph-space-after', '12');
      expect(text.getText()!.paragraph.spaceAfter, 12);
    });

    testWidgets('a restyle keeps the words where they are', (tester) async {
      final p = withComp();
      final text = p.comp.addTextLayer();
      p.uiState.selectedLayer.value = text;
      await mount(tester, p, const ParagraphPanelFrb());

      // Right-aligned text holds its right edge, so the anchor follows the
      // edge across to the other side of the words.
      final before = anchorX(text);
      await tester.tap(find.byKey(const ValueKey('paragraph-align-right')));
      await tester.pump();
      final block = measureText(
        text: 'Text',
        size: 72,
        style: plainTextStyle,
        paragraph:
            plainParagraphStyle.copyWith(align: BridgeTextAlign.right),
        animated: false,
      );
      expect(anchorX(text) - before, closeTo(block.right - block.left, 12),
          reason: 'the anchor moved from the left edge to the right one');
    });
  });
}
