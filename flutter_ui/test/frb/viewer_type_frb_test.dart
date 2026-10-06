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
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/state/layer_bounds.dart';
import 'package:lumit_flutter/state/tools.dart';

import 'frb_test_support.dart';

// The line from the report.
const _words = 'Hello, this is a text';
const _size = 72.0;

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
    final line = measuredTextLine(_words, _size);
    return screenOf(tester, box, line.carets[i], line.baseline - _size * 0.3);
  }

  TextEditingController field(WidgetTester tester) =>
      tester.widget<EditableText>(find.byType(EditableText)).controller;

  group('Type tool (frb)', () {
    testWidgets('the box round a line is the line the engine draws',
        (tester) async {
      final p = withLine();
      await mount(tester, p);

      final line = measureTextLine(text: _words, size: _size, animated: false);
      final box = boxOf(tester, p.layer);
      expect(box.bounds.width, line.width,
          reason: 'the raster the engine draws the words into');
      expect(box.bounds.height, line.height);
      expect(box.bounds.width, lessThan(_words.length * _size * 0.5 * 0.9),
          reason: 'well inside the old half-a-size-per-letter estimate');
      expect(line.carets.length, _words.length + 1);
      expect(line.carets.last, closeTo(line.width, 1),
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
  });
}
