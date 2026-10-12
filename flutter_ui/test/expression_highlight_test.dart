// An expression's text is coloured by the grammar of the language it is
// written in. JavaScript's comes with the highlighter; Rhai's is Lumit's own
// file, so this is the test that it loads and tells a line's parts apart.

import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart'
    show ExpressionTextEditingController;
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/state/expression_language.dart';
import 'package:syntax_highlight/syntax_highlight.dart';

void main() {
  testWidgets('JavaScript and Rhai are each coloured by their own grammar',
      (tester) async {
    await tester
        .runAsync(ExpressionTextEditingController.initSyntaxHighlighting);
    final theme = ExpressionTextEditingController.darkTheme!;

    /// The colour each piece of [text] is drawn in, by the piece.
    Map<String, Color?> colours(BridgeExpressionLanguage language, String text) {
      final out = <String, Color?>{};
      void walk(InlineSpan span) {
        if (span is! TextSpan) return;
        final piece = (span.text ?? '').trim();
        if (piece.isNotEmpty) out[piece] = span.style?.color;
        span.children?.forEach(walk);
      }

      walk(Highlighter(language: expressionGrammar(language), theme: theme)
          .highlight(text));
      return out;
    }

    final js = colours(BridgeExpressionLanguage.javaScript,
        'var amp = 20; // how far\nvalue + wiggle(3, amp) + "px"');
    expect(js.values.toSet().length, greaterThanOrEqualTo(4),
        reason: 'a keyword, a number, a comment and a string: $js');

    final rhai = colours(BridgeExpressionLanguage.rhai,
        'if time > 2.0 { sin(time) * 50 } else { "still" } // wave');
    expect(rhai['if'], isNot(rhai['time']), reason: 'a keyword stands out');
    expect(rhai['sin'], isNot(rhai['time']), reason: 'so does a function');
    expect(rhai['50'], isNot(rhai['time']), reason: 'and a number');
    expect(rhai['"still"'], isNot(rhai['50']));
    expect(rhai['// wave'], isNot(rhai['"still"']));
  });
}
