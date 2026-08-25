import 'package:flutter/material.dart';
import 'package:lumit_flutter/data/expressions_metadata.dart';
import 'package:lumit_flutter/theme/theme.dart';

class ExpressionsSuggestion {
  FunctionDef function;

  ExpressionsSuggestion(this.function);
}

/// Completion for the expression editor: suggests the API's functions for the
/// word under the cursor, and knows how to draw and apply one. The one
/// autofill source there is — [HouseTextField] takes it directly.
class ExpressionAutofillGenerator {
  void applySuggestion(
      ExpressionsSuggestion suggestion, TextEditingController controller) {
    var replacement = suggestion.function.name;

    if (suggestion.function.isGetter) {
      replacement = ".${replacement.split(".").last}";
    }

    var area = getCurrentWord(controller.text, controller.selection.baseOffset);

    controller.text =
        controller.text.replaceRange(area.$2, area.$3, replacement);

    var caret = area.$2 + replacement.length;
    controller.selection =
        TextSelection(baseOffset: caret, extentOffset: caret);
  }

  Widget buildSuggestion(ExpressionsSuggestion suggestion, LumitTheme theme) {
    var t = theme;
    var data = suggestion.function;

    String signature = data.signature.replaceFirst(data.name, "");
    if (data.isGetter) {
      signature = " ->${signature.split("->").last}";
    }
    return Row(
      spacing: 8,
      mainAxisSize: MainAxisSize.min,
      children: [
        Row(
          children: [
            Text(
              data.name,
              style: t.mono.copyWith(color: t.textPrimary),
            ),
            Text(
              signature,
              style: t.mono,
            ),
          ],
        ),
        Column(
            children: data.docComments
                .map((i) => Text(i.replaceFirst("///", ""), style: t.small))
                .toList()),
      ],
    );
  }

  (String, int, int) getCurrentWord(String text, int cursor) {
    if (cursor < 0 || cursor > text.length) {
      return ('', 0, 0);
    }

    final isWordChar = RegExp(r'[A-Za-z_.]');

    if (cursor < text.length && !isWordChar.hasMatch(text[cursor])) {
      if (cursor == 0 || !isWordChar.hasMatch(text[cursor - 1])) {
        return ('', 0, 0);
      }
    }

    int start = cursor;
    int end = cursor;

    if (start > 0 &&
        (start == text.length || !isWordChar.hasMatch(text[start])) &&
        isWordChar.hasMatch(text[start - 1])) {
      start--;
      end--;
    }

    while (start > 0 && isWordChar.hasMatch(text[start - 1])) {
      start--;
    }

    while (end < text.length && isWordChar.hasMatch(text[end])) {
      end++;
    }

    return (text.substring(start, end), start, end);
  }

  List<ExpressionsSuggestion> getSuggestions(String text, int cursor) {
    var word = getCurrentWord(text, cursor);

    if (word.$1.isEmpty) {
      return [];
    }

    var suggestions = ExpressionsMetadata.api.functions
        .where((i) => i.name.contains(word.$1))
        .map((f) => ExpressionsSuggestion(f))
        .toList();

    suggestions.sort((a, b) => a.function.name.compareTo(b.function.name));

    return suggestions;
  }
}
