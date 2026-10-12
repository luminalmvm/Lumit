// Which language an expression is written in, as the app shows and stores it.
//
// The engine keeps the choice beside each expression and never works it out
// from the text, so every place an expression is written has to say which
// language it is in. This file is what those places share: the name on screen,
// the word a settings file keeps, and the picker.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';

/// The name a language goes by on screen.
String expressionLanguageLabel(BridgeExpressionLanguage language) =>
    switch (language) {
      BridgeExpressionLanguage.rhai => l10n.expressionLanguageRhai,
      BridgeExpressionLanguage.javaScript => l10n.expressionLanguageJavaScript,
    };

/// The word a settings file keeps a language as.
String expressionLanguageId(BridgeExpressionLanguage language) =>
    switch (language) {
      BridgeExpressionLanguage.rhai => 'rhai',
      BridgeExpressionLanguage.javaScript => 'javascript',
    };

/// The language a stored word names. Anything else is Rhai, as nothing at all
/// is, which is what every expression was before there was a choice.
BridgeExpressionLanguage expressionLanguageOfId(Object? id) =>
    id == 'javascript'
        ? BridgeExpressionLanguage.javaScript
        : BridgeExpressionLanguage.rhai;

/// The grammar a language's text is coloured with, by the name the
/// highlighter knows it under.
String expressionGrammar(BridgeExpressionLanguage language) =>
    switch (language) {
      BridgeExpressionLanguage.rhai => 'rhai',
      BridgeExpressionLanguage.javaScript => 'javascript',
    };

/// The languages in the order every list of them is drawn.
const List<BridgeExpressionLanguage> expressionLanguages = [
  BridgeExpressionLanguage.javaScript,
  BridgeExpressionLanguage.rhai,
];

/// The dropdown that picks an expression's language. [dense] is the in-row
/// face, for a bar or a panel's foot.
class ExpressionLanguagePicker extends StatelessWidget {
  final BridgeExpressionLanguage value;
  final ValueChanged<BridgeExpressionLanguage> onChanged;
  final bool dense;

  const ExpressionLanguagePicker({
    super.key,
    required this.value,
    required this.onChanged,
    this.dense = false,
  });

  @override
  Widget build(BuildContext context) => BareDropdown<BridgeExpressionLanguage>(
        value: value,
        options: expressionLanguages,
        label: expressionLanguageLabel,
        dense: dense,
        onChanged: onChanged,
      );
}
