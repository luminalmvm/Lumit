// The Type tool on frb, against the real engine: the box round a line of
// text, the caret, the selection, and who has the keyboard.
//
// Each of these was a bug report. The box ran well past the end of the words,
// because it was half the point size per character. The caret stood at the
// end of that box, not the end of the words. Nothing could be selected by
// dragging. And the first click into a finished line put a caret down without
// giving it the keyboard, so the next letter typed ran a shortcut (`S` for
// Scale).

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/viewer_gizmo.dart' show LayerBox;
import 'package:lumit_flutter/panels/viewer_panel_frb.dart';
import 'package:lumit_flutter/panels/viewer_type.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/state/text_documents.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/tools.dart';

import 'frb_test_support.dart';

// The line from the report.
const _words = 'Hello, this is a text';
const _size = 72.0;

/// The test's line as the engine lays it out, unstyled.
BridgeTextBlock _measured() => measureText(
      text: _words,
      size: _size,
      style: plainTextStyle,
      paragraph: plainParagraphStyle,
      animated: false,
    );

void main() {
  setUpAll(initEngineForTests);

  ({
    LumitState state,
    LumitUiState uiState,
    CompositionReference comp,
    LayerReference layer,
  }) withLine() {
    final p = freshProject();
    final comp = p.state.project!.newComposition(name: 'Scene');
    final layer = comp.addTextLayerAt(
      document: const BridgeTextDocument(
        style: plainTextStyle,
        paragraph: plainParagraphStyle,
        text: _words,
        size: _size,
        fill: BridgeColourRgba(r: 1, g: 1, b: 1, a: 1),
        pathOffset: BridgeScalar.static_(0),
        animators: [],
      ),
      x: 400,
      y: 500,
    );
    p.uiState.setSelectedComp(comp);
    p.uiState.tools.select(ToolMode.typeHorizontal);
    p.uiState.model.refresh();
    return (state: p.state, uiState: p.uiState, comp: comp, layer: layer);
  }

  Future<void> mount(WidgetTester tester, dynamic p) async {
    await tester.pumpWidget(hostPanel(
      child: const ViewerPanelFrb(),
      state: p.state as LumitState,
      uiState: p.uiState as LumitUiState,
      size: const Size(900, 600),
    ));
    await tester.pump();
  }

  LayerBox boxOf(WidgetTester tester, LayerReference layer) => tester
      .widget<ViewerTypeLayer>(find.byType(ViewerTypeLayer))
      .boxes
      .firstWhere((b) => b.id == layer.internallayerId);

  /// Where layer pixel (x, y) is on the test's screen.
  Offset screenOf(WidgetTester tester, LayerBox box, double x, double y) =>
      tester.getTopLeft(find.byType(ViewerTypeLayer)) + box.map.toScreen(x, y);

  /// The point on screen just before character [i], halfway up a capital.
  Offset gap(WidgetTester tester, LayerBox box, int i) {
    final line = _measured().lines.first;
    return screenOf(tester, box, line.carets[i], line.baseline - _size * 0.3);
  }

  TextEditingController field(WidgetTester tester) =>
      tester.widget<EditableText>(find.byType(EditableText)).controller;

  group('Type tool (frb)', () {
    testWidgets('the box round a line is the line the engine draws',
        (tester) async {
      final p = withLine();
      await mount(tester, p);

      final block = _measured();
      final line = block.lines.first;
      final box = boxOf(tester, p.layer);
      expect(box.bounds.width, block.width,
          reason: 'the raster the engine draws the words into');
      expect(box.bounds.height, block.height);
      expect(box.bounds.width, lessThan(_words.length * _size * 0.5 * 0.9),
          reason: 'well inside the old half-a-size-per-letter estimate');
      expect(line.carets.length, _words.length + 1);
      expect(line.carets.last, closeTo(block.width, 1),
          reason: 'the caret after the last letter is on the box\'s edge');
    });

    testWidgets(
        'a click into a finished line takes the keyboard at once, with the'
        ' caret where it landed', (tester) async {
      final p = withLine();
      await mount(tester, p);

      await tester.tapAt(gap(tester, boxOf(tester, p.layer), 4));
      await tester.pump();

      expect(FocusManager.instance.primaryFocus?.debugLabel, 'Type tool',
          reason: 'one click: the next letter typed is text, not a shortcut');
      expect(field(tester).selection, const TextSelection.collapsed(offset: 4),
          reason: 'the caret goes in the gap clicked, not after the line');
      // As Windows, since a desktop field selects all its text on focus.
    }, variant: TargetPlatformVariant.only(TargetPlatform.windows));

    testWidgets('dragging across the words selects them, and pans nothing',
        (tester) async {
      final p = withLine();
      await mount(tester, p);
      final box = boxOf(tester, p.layer);
      Rect picture() =>
          tester.widget<ViewerTypeLayer>(find.byType(ViewerTypeLayer)).fitted;
      final before = picture();

      final drag = await tester.startGesture(gap(tester, box, 0),
          kind: PointerDeviceKind.mouse, buttons: kPrimaryButton);
      await tester.pump();
      await drag.moveTo(gap(tester, box, 2));
      await tester.pump();
      await drag.moveTo(gap(tester, box, 5));
      await tester.pump();
      await drag.up();
      await tester.pump();

      expect(field(tester).selection,
          const TextSelection(baseOffset: 0, extentOffset: 5));
      expect(FocusManager.instance.primaryFocus?.debugLabel, 'Type tool');
      expect(picture(), before,
          reason: 'a drag on the words is a selection, not a pan');
    });

    testWidgets('a double-click selects a word, a triple-click the line',
        (tester) async {
      final p = withLine();
      await mount(tester, p);
      final at = gap(tester, boxOf(tester, p.layer), 8);

      await tester.tapAt(at);
      await tester.pump(const Duration(milliseconds: 50));
      await tester.tapAt(at);
      await tester.pump(const Duration(milliseconds: 50));
      expect(field(tester).selection,
          const TextSelection(baseOffset: 7, extentOffset: 11));

      await tester.tapAt(at);
      await tester.pump(const Duration(milliseconds: 50));
      expect(field(tester).selection,
          const TextSelection(baseOffset: 0, extentOffset: _words.length));
      await tester.pump(const Duration(seconds: 1));
    });

    testWidgets('Shift-click stretches the selection from the caret',
        (tester) async {
      final p = withLine();
      await mount(tester, p);
      final box = boxOf(tester, p.layer);

      await tester.tapAt(gap(tester, box, 2));
      await tester.pump(const Duration(seconds: 1));
      await simulateKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.tapAt(gap(tester, box, 7));
      await simulateKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.pump();

      expect(field(tester).selection,
          const TextSelection(baseOffset: 2, extentOffset: 7));
    });

    testWidgets(
        'Shift+Enter breaks the line, the arrows move by the engine\'s lines,'
        ' and the style survives the edit', (tester) async {
      final p = withLine();
      // Tracked out, so the edit has a style to lose.
      final styled = p.layer.getText()!;
      p.layer.setText(
        document: styled.copyWith(
            style: styled.style.copyWith(tracking: 40)),
      );
      p.uiState.model.refresh();
      await mount(tester, p);

      // The gap before character 5, as the styled line is laid out.
      final document = p.layer.getText()!;
      final line = measureText(
        text: document.text,
        size: document.size,
        style: document.style,
        paragraph: document.paragraph,
        animated: false,
      ).lines.first;
      await tester.tapAt(screenOf(tester, boxOf(tester, p.layer),
          line.carets[5], line.baseline - _size * 0.3));
      await tester.pump(const Duration(seconds: 1));
      expect(field(tester).selection, const TextSelection.collapsed(offset: 5));

      await simulateKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await simulateKeyDownEvent(LogicalKeyboardKey.enter);
      await simulateKeyUpEvent(LogicalKeyboardKey.enter);
      await simulateKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.pump();
      expect(field(tester).text, 'Hello\n, this is a text');
      expect(field(tester).selection, const TextSelection.collapsed(offset: 6),
          reason: 'the caret starts the new line');

      // Up from the start of the second line is the start of the first, and
      // End is after its last letter, before the break.
      await simulateKeyDownEvent(LogicalKeyboardKey.arrowUp);
      await simulateKeyUpEvent(LogicalKeyboardKey.arrowUp);
      expect(field(tester).selection, const TextSelection.collapsed(offset: 0));
      await simulateKeyDownEvent(LogicalKeyboardKey.end);
      await simulateKeyUpEvent(LogicalKeyboardKey.end);
      expect(field(tester).selection, const TextSelection.collapsed(offset: 5));
      await simulateKeyDownEvent(LogicalKeyboardKey.arrowDown);
      await simulateKeyUpEvent(LogicalKeyboardKey.arrowDown);
      expect(field(tester).selection.baseOffset, greaterThan(6),
          reason: 'down a line, at about the same x');

      // Enter still ends the edit, and writes both lines with the style kept.
      await tester.testTextInput.receiveAction(TextInputAction.done);
      await tester.pump();
      final written = p.layer.getText()!;
      expect(written.text, 'Hello\n, this is a text');
      expect(written.style.tracking, 40);
      expect(
          measureText(
            text: written.text,
            size: written.size,
            style: written.style,
            paragraph: written.paragraph,
            animated: false,
          ).lines.length,
          2);
      await tester.pump(const Duration(seconds: 1));
    }, variant: TargetPlatformVariant.only(TargetPlatform.windows));
  });
}
