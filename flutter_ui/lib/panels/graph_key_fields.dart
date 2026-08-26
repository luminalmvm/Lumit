// The graph editor's numeric-entry popover (docs/07 §5.3). Split out of
// graph_editor_frb.dart, which re-exports it.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';

// ---------------------------------------------------------------------------
// Numeric entry (docs/07 §5.3).
// ---------------------------------------------------------------------------

/// The popover's width and its label column — the Ease popover's own two
/// measures, so the two small boxes the graph opens read as one family.
const double _keyFieldsWidth = 168;
const double _keyFieldsLabel = 46;

/// Open the **numeric entry** box on one keyframe: its exact frame, its exact
/// value, and how far each of its two eases reaches.
///
/// [onApply] is called with all four numbers on every change, so the caller
/// writes one key rather than four separate edits, and the box stays up — the
/// point of typing numbers is usually to type more than one. It carries no
/// buttons for the same reason every value well in the application carries
/// none: a field commits what it holds.
Future<void> showKeyFieldsPopover({
  required BuildContext context,
  required Offset position,
  required double frame,
  required double value,
  required double inPercent,
  required double outPercent,
  required double minFrame,
  required double maxFrame,
  required void Function(
          double frame, double value, int inPercent, int outPercent)
      onApply,
}) =>
    showLumitPopup<void>(
      context: context,
      position: position,
      builder: (close) => _KeyFieldsPopover(
        frame: frame,
        value: value,
        inPercent: inPercent,
        outPercent: outPercent,
        minFrame: minFrame,
        maxFrame: maxFrame,
        onApply: onApply,
      ),
    );

class _KeyFieldsPopover extends StatefulWidget {
  final double frame;
  final double value;
  final double inPercent;
  final double outPercent;
  final double minFrame;
  final double maxFrame;
  final void Function(double frame, double value, int inPercent, int outPercent)
      onApply;

  const _KeyFieldsPopover({
    required this.frame,
    required this.value,
    required this.inPercent,
    required this.outPercent,
    required this.minFrame,
    required this.maxFrame,
    required this.onApply,
  });

  @override
  State<_KeyFieldsPopover> createState() => _KeyFieldsPopoverState();
}

class _KeyFieldsPopoverState extends State<_KeyFieldsPopover> {
  /// The four numbers as the box holds them. Kept here rather than read back
  /// off the key, because the box outlives the channel snapshot it opened on
  /// (see `_applyKeyFields`): what it shows is what was typed into it.
  late double _frame = widget.frame;
  late double _value = widget.value;
  late double _in = widget.inPercent;
  late double _out = widget.outPercent;

  void _apply() => widget.onApply(_frame, _value, _in.round(), _out.round());

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      width: _keyFieldsWidth,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _row(t, l10n.graphKeyFrameField, 'graph-fields-frame', _frame,
              min: widget.minFrame,
              max: widget.maxFrame,
              decimals: 0, set: (v) {
            _frame = v;
            _apply();
          }),
          _row(t, l10n.graphKeyValueField, 'graph-fields-value', _value,
              min: -100000, max: 100000, decimals: 2, set: (v) {
            _value = v;
            _apply();
          }),
          _row(t, l10n.graphEaseIn, 'graph-fields-in', _in,
              min: 0,
              max: 100,
              decimals: 0,
              suffix: l10n.unitSymbolPercent, set: (v) {
            _in = v;
            _apply();
          }),
          _row(t, l10n.graphEaseOut, 'graph-fields-out', _out,
              min: 0,
              max: 100,
              decimals: 0,
              suffix: l10n.unitSymbolPercent, set: (v) {
            _out = v;
            _apply();
          }),
        ],
      ),
    );
  }

  Widget _row(
    LumitTheme t,
    String label,
    String key,
    double value, {
    required num min,
    required num max,
    required int decimals,
    String? suffix,
    required ValueChanged<double> set,
  }) =>
      Padding(
        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 2),
        child: Row(
          children: [
            SizedBox(
              width: _keyFieldsLabel,
              child: Text(label,
                  style: t.body, maxLines: 1, overflow: TextOverflow.ellipsis),
            ),
            const SizedBox(width: 6),
            Expanded(
              child: DragValueField(
                key: ValueKey<String>(key),
                value: value,
                min: min,
                max: max,
                decimals: decimals,
                suffix: suffix,
                keyed: true,
                onChanged: (v) => setState(() => set(v.toDouble())),
              ),
            ),
          ],
        ),
      );
}
