// Animation ▸ Add expression (on the shared dialogue pattern).
//
// In plain terms: instead of keyframes, a property can be given a small
// program that works its value out every frame — the language is Rhai, and the
// source is stored in the document exactly as it is typed.
//
// A code well rather than a one-line field: an expression is written in lines,
// and Enter inside it makes a new one. The dialogue's own Apply is what
// commits, which is why the button is the only way out that keeps the text.
//
// It decides nothing. It collects a string; the menu row writes it onto every
// picked property.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';

/// Ask for an expression, seeded with [initial]. Completes with null when
/// dismissed, and with the text — which may be blank — when applied.
Future<String?> showExpressionDialogFrb({
  required BuildContext context,
  String initial = '',
}) =>
    showLumitModal<String>(
      context: context,
      id: 'expression',
      initialSize: const Size(460, 300),
      minSize: const Size(320, 220),
      builder: (close) => _ExpressionBody(
        initial: initial,
        onConfirm: close,
        onCancel: () => close(null),
      ),
    );

class _ExpressionBody extends StatefulWidget {
  final String initial;
  final ValueChanged<String> onConfirm;
  final VoidCallback onCancel;

  const _ExpressionBody({
    required this.initial,
    required this.onConfirm,
    required this.onCancel,
  });

  @override
  State<_ExpressionBody> createState() => _ExpressionBodyState();
}

class _ExpressionBodyState extends State<_ExpressionBody> {
  late final TextEditingController _text =
      TextEditingController(text: widget.initial);

  @override
  void dispose() {
    _text.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
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
          const SizedBox(height: 16),
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              HouseButton(
                key: const ValueKey('expression-confirm'),
                primary: true,
                onPressed: () => widget.onConfirm(_text.text),
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
