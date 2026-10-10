// Animation ▸ Add expression (on the shared dialogue pattern).
//
// In plain terms: instead of keyframes, a property can be given a small
// program that works its value out every frame. It is written in JavaScript or
// in Rhai, whichever the dropdown under the text says, and the source is
// stored in the document exactly as it is typed.
//
// A code well rather than a one-line field: an expression is written in lines,
// and Enter inside it makes a new one. The dialogue's own Apply is what
// commits, which is why the button is the only way out that keeps the text.
//
// It decides nothing. It collects a text and the language it is in; the menu
// row writes them onto every picked property.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';

import '../l10n/strings.dart';
import '../panels/effect_param_row_frb.dart'
    show ExpressionTextEditingController;
import '../state/expression_language.dart';
import '../widgets/controls.dart';

/// What the dialogue hands back: the text, which may be blank, and the
/// language it is written in.
typedef ExpressionDraft = ({String text, BridgeExpressionLanguage language});

/// Ask for an expression, seeded with [initial] in [language]. Completes with
/// null when dismissed, and with the draft when applied.
Future<ExpressionDraft?> showExpressionDialogFrb({
  required BuildContext context,
  required BridgeExpressionLanguage language,
  String initial = '',
}) =>
    showLumitModal<ExpressionDraft>(
      context: context,
      id: 'expression',
      initialSize: const Size(460, 324),
      minSize: const Size(320, 244),
      builder: (close) => _ExpressionBody(
        initial: initial,
        language: language,
        onConfirm: close,
        onCancel: () => close(null),
      ),
    );

class _ExpressionBody extends StatefulWidget {
  final String initial;
  final BridgeExpressionLanguage language;
  final ValueChanged<ExpressionDraft> onConfirm;
  final VoidCallback onCancel;

  const _ExpressionBody({
    required this.initial,
    required this.language,
    required this.onConfirm,
    required this.onCancel,
  });

  @override
  State<_ExpressionBody> createState() => _ExpressionBodyState();
}

class _ExpressionBodyState extends State<_ExpressionBody> {
  late final ExpressionTextEditingController _text =
      ExpressionTextEditingController(
          text: widget.initial, language: expressionGrammar(widget.language));
  late BridgeExpressionLanguage _language = widget.language;

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // The text is coloured in the language the dropdown shows.
    _text.language = expressionGrammar(_language);
    return FloatSurface(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.only(bottom: 10),
            child: Text(l10n.menuAddExpression, style: t.bodyPrimary),
          ),
          SizedBox(
            height: 140,
            child: HouseTextField(
              key: const ValueKey('expression-text'),
              controller: _text,
              width: double.infinity,
              autofocus: true,
              multiline: true,
              style: t.mono,
              hint: l10n.expressionHint,
            ),
          ),
          // The language sits under the text at its right-hand end, where an
          // editor's status line keeps it.
          Padding(
            padding: const EdgeInsets.only(top: 6),
            child: Align(
              alignment: Alignment.centerRight,
              child: SizedBox(
                width: 120,
                height: 22,
                child: ExpressionLanguagePicker(
                  key: const ValueKey('expression-language'),
                  value: _language,
                  onChanged: (language) =>
                      setState(() => _language = language),
                ),
              ),
            ),
          ),
          const SizedBox(height: 12),
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              HouseButton(
                key: const ValueKey('expression-confirm'),
                primary: true,
                onPressed: () => widget
                    .onConfirm((text: _text.text, language: _language)),
                child: Text(l10n.apply),
              ),
              const SizedBox(width: 8),
              HouseButton(
                key: const ValueKey('expression-cancel'),
                onPressed: widget.onCancel,
                child: Text(l10n.cancel),
              ),
            ],
          ),
        ],
      ),
    );
  }
}
