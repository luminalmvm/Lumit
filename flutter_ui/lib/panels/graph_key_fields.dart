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
          keyFieldRow(t, l10n.graphKeyFrameField, 'graph-fields-frame', _frame,
              min: widget.minFrame,
              max: widget.maxFrame,
              decimals: 0,
              set: (v) => setState(() {
                    _frame = v;
                    _apply();
                  })),
          keyFieldRow(t, l10n.graphKeyValueField, 'graph-fields-value', _value,
              min: -100000,
              max: 100000,
              decimals: 2,
              set: (v) => setState(() {
                    _value = v;
                    _apply();
                  })),
          keyFieldRow(t, l10n.graphEaseIn, 'graph-fields-in', _in,
              min: 0,
              max: 100,
              decimals: 0,
              suffix: l10n.unitSymbolPercent,
              set: (v) => setState(() {
                    _in = v;
                    _apply();
                  })),
          keyFieldRow(t, l10n.graphEaseOut, 'graph-fields-out', _out,
              min: 0,
              max: 100,
              decimals: 0,
              suffix: l10n.unitSymbolPercent,
              set: (v) => setState(() {
                    _out = v;
                    _apply();
                  })),
        ],
      ),
    );
  }
}

/// Open the box on one **speed point**: its playback speed in per cent, with
/// the editor already up and the number selected, so the next keys typed are
/// the new speed. `Enter` writes it and shuts the box, `Escape` shuts it and
/// writes nothing.
///
/// The graph's speed envelope and the sequence view's open the same box. The
/// graph gives [frame] too, and the point's frame is then a second well above
/// the speed, held between [minFrame] and [maxFrame].
///
/// [onApply] hears the speed, and the frame where there is one. It is not
/// called for a speed left as it was shown: the well reads to two places, and
/// writing that back would round a point somebody had put somewhere finer.
Future<void> showSpeedFieldPopover({
  required BuildContext context,
  required Offset position,
  required double percent,
  double? frame,
  double minFrame = 0,
  double maxFrame = 0,
  required void Function(double percent, double? frame) onApply,
}) =>
    showLumitPopup<void>(
      context: context,
      position: position,
      builder: (close) => _SpeedFieldPopover(
        percent: percent,
        frame: frame,
        minFrame: minFrame,
        maxFrame: maxFrame,
        onApply: onApply,
        onDone: () => close(null),
      ),
    );

class _SpeedFieldPopover extends StatefulWidget {
  final double percent;
  final double? frame;
  final double minFrame;
  final double maxFrame;
  final void Function(double percent, double? frame) onApply;
  final VoidCallback onDone;

  const _SpeedFieldPopover({
    required this.percent,
    required this.frame,
    required this.minFrame,
    required this.maxFrame,
    required this.onApply,
    required this.onDone,
  });

  @override
  State<_SpeedFieldPopover> createState() => _SpeedFieldPopoverState();
}

class _SpeedFieldPopoverState extends State<_SpeedFieldPopover> {
  /// How many places the speed well reads to, the Clip speed dialogue's own.
  static const int _places = 2;

  late double _percent = widget.percent;
  late double? _frame = widget.frame;

  /// A scope of its own, so the well takes the keyboard as the box opens
  /// whatever held it before, and hands it back when the box shuts.
  final FocusScopeNode _scope = FocusScopeNode(debugLabel: 'speed field');

  @override
  void dispose() {
    _scope.dispose();
    super.dispose();
  }

  void _setSpeed(double v) {
    if (v.toStringAsFixed(_places) == _percent.toStringAsFixed(_places)) return;
    setState(() => _percent = v);
    widget.onApply(v, _frame);
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FocusScope(
      node: _scope,
      child: FloatSurface(
        width: _keyFieldsWidth,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (_frame case final frame?)
              keyFieldRow(t, l10n.graphKeyFrameField, 'speed-field-frame', frame,
                  min: widget.minFrame,
                  max: widget.maxFrame,
                  decimals: 0,
                  set: (v) {
                    setState(() => _frame = v);
                    widget.onApply(_percent, v);
                  }),
            keyFieldRow(t, l10n.graphKeySpeedField, 'speed-field', _percent,
                // Either way round, and well past anything footage survives.
                min: -10000,
                max: 10000,
                decimals: _places,
                suffix: l10n.unitSymbolPercent,
                autofocus: true,
                submitted: widget.onDone,
                set: _setSpeed),
          ],
        ),
      ),
    );
  }
}

/// One labelled well, the row every small box of key numbers is made of: the
/// label in a fixed column, the well taking the rest. [set] hears a committed
/// number, and [live] hears each tick of a well drag, for a caller that shows the
/// number as it goes and writes it once on release. [autofocus] opens the well
/// ready to type into, and [submitted] hears `Enter` after [set] has.
Widget keyFieldRow(
  LumitTheme t,
  String label,
  String key,
  double value, {
  required num min,
  required num max,
  required int decimals,
  String? suffix,
  double labelWidth = _keyFieldsLabel,
  bool autofocus = false,
  VoidCallback? submitted,
  ValueChanged<double>? live,
  required ValueChanged<double> set,
}) =>
    Padding(
      padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 2),
      child: Row(
        children: [
          SizedBox(
            width: labelWidth,
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
              autofocus: autofocus,
              onSubmitted: submitted,
              onChangeLive: live == null ? null : (v) => live(v.toDouble()),
              onChanged: (v) => set(v.toDouble()),
            ),
          ),
        ],
      ),
    );
