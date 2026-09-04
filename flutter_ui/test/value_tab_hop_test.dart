// Tab hops between value wells, and the well it lands on is ready to type in
// (15-DESIGN §12A.3).
//
// Two halves, both binding behaviour of every value field rather than of any
// one panel: a well reached by keyboard traversal **opens its editor with the
// whole value selected**, so the first keystroke replaces it; and the hop
// **runs on past the end of a row** into the next row's first well, with
// `Shift+Tab` mirroring back up.

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

/// Two rows of two wells, which is what a Position row above a Scale row is:
/// the shape the hop has to cross.
Widget _harness() => MaterialApp(
      home: ThemeScope(
        theme: LumitTheme.dark(),
        animationLevel: AnimationLevel.none,
        showTooltips: false,
        child: Overlay(
          initialEntries: [
            OverlayEntry(
              builder: (_) => Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  for (var row = 0; row < 2; row++)
                    Row(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        for (var col = 0; col < 2; col++)
                          DragValueField(
                            key: ValueKey('r${row}c$col'),
                            value: row * 10 + col,
                            min: 0,
                            max: 100,
                            onChanged: (_) {},
                          ),
                      ],
                    ),
                ],
              ),
            ),
          ],
        ),
      ),
    );

/// The well whose editor is open, by its key — null while none is.
String? _editing(WidgetTester tester) {
  final open = find.byType(EditableText).evaluate();
  if (open.isEmpty) return null;
  final field = tester.firstWidget<DragValueField>(find.ancestor(
    of: find.byType(EditableText),
    matching: find.byType(DragValueField),
  ));
  return (field.key as ValueKey<String>).value;
}

Future<void> _tab(WidgetTester tester, {bool shift = false}) async {
  if (shift) await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
  await tester.sendKeyEvent(LogicalKeyboardKey.tab);
  if (shift) await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
  await tester.pumpAndSettle();
}

void main() {
  testWidgets('a well reached by Tab arrives with its value selected',
      (tester) async {
    await tester.pumpWidget(_harness());
    await _tab(tester);

    expect(_editing(tester), 'r0c0');
    final editor = tester.widget<EditableText>(find.byType(EditableText));
    expect(editor.controller.text, '0');
    expect(editor.controller.selection,
        const TextSelection(baseOffset: 0, extentOffset: 1));
  });

  testWidgets('Tab off a row\'s last well lands on the next row\'s first',
      (tester) async {
    await tester.pumpWidget(_harness());
    await _tab(tester);
    expect(_editing(tester), 'r0c0');
    await _tab(tester);
    expect(_editing(tester), 'r0c1');

    // The hop that matters: past the end of the row rather than out of the
    // panel.
    await _tab(tester);
    expect(_editing(tester), 'r1c0');
  });

  testWidgets('Shift+Tab mirrors back up into the row above', (tester) async {
    await tester.pumpWidget(_harness());
    for (var i = 0; i < 3; i++) {
      await _tab(tester);
    }
    expect(_editing(tester), 'r1c0');

    await _tab(tester, shift: true);
    expect(_editing(tester), 'r0c1');
    await _tab(tester, shift: true);
    expect(_editing(tester), 'r0c0');
  });
}
