// Clicking a value must not move the value.
//
// **Why this file exists.** The owner called entering edit mode on a value
// well "very jarring": the number jumped as the editor opened. Three separate
// causes, two in [DragValueField] and one in [TimeReadout] —
//
// * [DragValueField] rested at the width of its own reading with the number
//   against the right edge, and opened a **fixed 72-wide** box with the text
//   against the *left* edge. Both the box and the number moved, on every
//   click.
// * The open editor's box was the well's whole 20px height with its text at
//   the *top* of it, four and a half pixels above where the reading had been.
//   The same complaint on the other axis, found once the sideways jump was
//   gone: an `EditableText` handed a tight height lays its line out at the top.
// * [TimeReadout] kept its box, but rested against the left of a slot cut for
//   the longest reading it could ever carry. The frame count rests as `F48`
//   and edits as `48`, so the letter went and the digits slid one
//   glyph left to fill the gap.
//
// All three are now anchored the same way in both states, so the glyphs that
// survive the change stay on the pixels they were on.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';
import 'package:lumit_flutter/widgets/time_readout.dart';

Widget _harness(Widget child) => Directionality(
      textDirection: TextDirection.ltr,
      child: ThemeScope(
        theme: LumitTheme.dark(),
        animationLevel: AnimationLevel.none,
        showTooltips: false,
        child: Overlay(
          initialEntries: [
            OverlayEntry(builder: (_) => Center(child: child)),
          ],
        ),
      ),
    );

/// Where the open editor draws its text, and how wide its box is.
///
/// `EditableText` fills the box it is given, so the interesting number is the
/// box's right edge less its padding — which is exactly where a right-anchored
/// reading ends. Measured off the render object rather than the glyphs because
/// a caret and a selection sit in the same box.
Rect _editorBox(WidgetTester tester) =>
    tester.getRect(find.byType(EditableText));

void main() {
  testWidgets('a value well opens its editor where the number already was',
      (tester) async {
    await tester.pumpWidget(_harness(DragValueField(
      value: 48,
      min: 0,
      max: 1000,
      onChanged: (_) {},
    )));

    final resting = tester.getRect(find.text('48'));
    final restingWell = tester.getRect(find.byType(DragValueField));

    await tester.tap(find.text('48'));
    await tester.pumpAndSettle();

    expect(find.byType(EditableText), findsOneWidget,
        reason: 'the click opened the editor');
    final editingWell = tester.getRect(find.byType(DragValueField));
    expect(editingWell.width, closeTo(restingWell.width, 0.01),
        reason: 'the well is the width of its own reading in both states — it '
            'used to jump to a fixed 72');
    expect(editingWell.left, closeTo(restingWell.left, 0.01));

    // The editor fills the well inside the same padding the resting face
    // wears, and its text is right-anchored, so the number ends on the pixel
    // it ended on.
    expect(_editorBox(tester).right, closeTo(resting.right, 0.01),
        reason: 'the digits end where they ended');
  });

  testWidgets('a signed value with a unit keeps its box on the click',
      (tester) async {
    // `+45°` rests, `45` edits: the sign and the unit are the reading's, not
    // the field's. The box must still be the resting box.
    await tester.pumpWidget(_harness(DragValueField(
      value: 45,
      min: -180,
      max: 180,
      signed: true,
      suffix: '°',
      onChanged: (_) {},
    )));

    final restingWell = tester.getRect(find.byType(DragValueField));
    await tester.tap(find.text('+45°'));
    await tester.pumpAndSettle();

    expect(tester.getRect(find.byType(DragValueField)).width,
        closeTo(restingWell.width, 0.01),
        reason: 'the well does not resize under the pointer');
  });

  testWidgets('the frame count keeps its digits when its letter goes',
      (tester) async {
    final style = LumitTheme.dark().mono.copyWith(fontSize: 10);
    await tester.pumpWidget(_harness(TimeReadout(
      frame: 48,
      format: (f) => 'F$f',
      editFormat: (f) => '$f',
      parse: int.tryParse,
      widthChars: 5,
      style: style,
      onCommit: (_) {},
      minFrame: 0,
      maxFrame: 999,
      well: true,
    )));

    final resting = tester.getRect(find.text('F48'));
    final slot = tester.getRect(find.byType(TimeReadout));

    await tester.tap(find.text('F48'));
    await tester.pumpAndSettle();

    expect(tester.getRect(find.byType(TimeReadout)), slot,
        reason: 'the slot is cut for the longest reading and never moves');
    expect(_editorBox(tester).right, closeTo(resting.right, 0.01),
        reason: 'both states are anchored to the slot\'s right edge, so the '
            '`48` of `F48` does not slide left when the `F` goes');
  });
}
