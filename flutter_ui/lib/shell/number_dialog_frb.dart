// One number, asked for.
//
// The menu bar reaches values the Timeline shows as rows — a mask's feather, a
// keyframe's influence — and a menu row cannot carry a drag field. So it asks:
// the dialogue pattern's row (K-444) with a single well in it, Enter to apply
// and Escape to leave everything as it was.
//
// It decides nothing and knows nothing about what it is asking for. One
// dialogue rather than one per value, because "type a number" is one question
// however many places ask it, and three copies of it would be three chances to
// drift apart.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';

/// Ask for a number, seeded with [value]. Completes with null when dismissed.
///
/// [title] is what is being asked for — the property's own name, so the
/// dialogue reads as that row lifted off the Timeline.
Future<double?> askNumberFrb({
  required BuildContext context,
  required String title,
  required double value,
  double min = -1000000,
  double max = 1000000,
  int decimals = 2,
  String? suffix,
}) =>
    showLumitModal<double>(
      context: context,
      builder: (close) => _NumberBody(
        title: title,
        value: value,
        min: min,
        max: max,
        decimals: decimals,
        suffix: suffix,
        onConfirm: close,
        onCancel: () => close(null),
      ),
    );

class _NumberBody extends StatefulWidget {
  final String title;
  final double value;
  final double min;
  final double max;
  final int decimals;
  final String? suffix;
  final ValueChanged<double> onConfirm;
  final VoidCallback onCancel;

  const _NumberBody({
    required this.title,
    required this.value,
    required this.min,
    required this.max,
    required this.decimals,
    required this.suffix,
    required this.onConfirm,
    required this.onCancel,
  });

  @override
  State<_NumberBody> createState() => _NumberBodyState();
}

class _NumberBodyState extends State<_NumberBody> {
  late double _value = widget.value;

  void _confirm() => widget.onConfirm(_value);

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // The dialogue takes focus when it opens and Enter applies wherever that
    // focus sits — the rule every one of them follows (K-243).
    return Focus(
      autofocus: true,
      onKeyEvent: (_, event) {
        if (event is! KeyDownEvent) return KeyEventResult.ignored;
        if (event.logicalKey != LogicalKeyboardKey.enter &&
            event.logicalKey != LogicalKeyboardKey.numpadEnter) {
          return KeyEventResult.ignored;
        }
        _confirm();
        return KeyEventResult.handled;
      },
      child: FloatSurface(
        width: 300,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                SizedBox(
                    width: 110,
                    child: Text(widget.title, style: t.bodyPrimary)),
                Expanded(
                  child: DragValueField(
                    key: const ValueKey('number-value'),
                    value: _value,
                    min: widget.min,
                    max: widget.max,
                    decimals: widget.decimals,
                    suffix: widget.suffix,
                    onChanged: (v) => setState(() => _value = v.toDouble()),
                  ),
                ),
              ],
            ),
            const SizedBox(height: 16),
            Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                HouseButton(
                  key: const ValueKey('number-confirm'),
                  primary: true,
                  onPressed: _confirm,
                  child: Text(l10n.apply),
                ),
                const SizedBox(width: 8),
                HouseButton(
                  key: const ValueKey('number-cancel'),
                  onPressed: widget.onCancel,
                  child: Text(l10n.cancel),
                ),
              ],
            ),
          ],
        ),
      ),
    );
  }
}
