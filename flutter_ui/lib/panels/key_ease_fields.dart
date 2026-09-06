// One keyframe's speed and influence as numbers you type (docs/07 §5.3).
//
// In plain terms: a tangent handle carries two numbers, how fast the curve
// arrives or leaves and how far the handle reaches. Dragging the handle sets
// them by eye, this sets them exactly, which is how a cut is synced. The same
// four wells stand in three places: the Keyframe speed dialogue (the key's
// menu, and Animation ▸ Keyframe speed…), the small box the graph's readout
// pill opens, and the foot of the Easing panel while one key is selected.
//
// The fields report only the numbers an edit touched, as a [KeyEase] with the
// rest null, so a side that was not typed into is never rewritten - a side
// that was automatic stays automatic, a straight one stays straight.

import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'graph_key_fields.dart';
import 'graph_maths.dart';

/// What the Timeline publishes while exactly one keyframe is selected
/// (`LumitUiState.easingKey`): which key, the frame it sits on, the unit its
/// speed is in, its eases, and the write that changes them.
///
/// Equal when the numbers are: the Timeline republishes on every build and the
/// panel must not be woken for a claim that says what the last one said.
class KeyEaseClaim {
  final String channelId;
  final int index;
  final int frame;
  final String? unit;
  final KeyEase ease;
  final void Function(String channelId, int index, KeyEase edit) apply;

  const KeyEaseClaim({
    required this.channelId,
    required this.index,
    required this.frame,
    required this.unit,
    required this.ease,
    required this.apply,
  });

  void write(KeyEase edit) => apply(channelId, index, edit);

  @override
  bool operator ==(Object other) =>
      other is KeyEaseClaim &&
      other.channelId == channelId &&
      other.index == index &&
      other.frame == frame &&
      other.unit == unit &&
      other.ease == ease;

  @override
  int get hashCode => Object.hash(channelId, index, frame, unit, ease);
}

/// Open the ease fields on one key as a small box beside [position], writing
/// each change through [onApply] as it is made - the readout pill's box.
Future<void> showKeyEasePopover({
  required BuildContext context,
  required Offset position,
  required KeyEase ease,
  required String? unit,
  required ValueChanged<KeyEase> onApply,
}) =>
    showLumitPopup<void>(
      context: context,
      position: position,
      builder: (close) => FloatSurface(
        width: 200,
        child: KeyEaseFields(
          ease: ease,
          unit: unit,
          keyPrefix: 'graph-ease',
          onChanged: onApply,
        ),
      ),
    );

/// The four wells, and the tick that keeps the two speeds one.
///
/// [ease] is what the wells open on, and a side with no numbers in it is not
/// drawn, which is how an end key shows one side. When [ease] changes under
/// the widget - a handle dragged while the panel shows the key - the wells
/// take the new numbers, so they always read what the curve holds.
///
/// [onChanged] hears each committed edit with only its numbers filled in. A
/// well drag reports once, on release, the number moving in the well as it goes.
class KeyEaseFields extends StatefulWidget {
  final KeyEase ease;
  final String? unit;
  final ValueChanged<KeyEase> onChanged;

  /// The wells' widget keys: `<prefix>-speed-in`, `-influence-in`,
  /// `-speed-out`, `-influence-out`, and `<prefix>-continuous` for the tick.
  final String keyPrefix;

  const KeyEaseFields({
    super.key,
    required this.ease,
    required this.unit,
    required this.onChanged,
    required this.keyPrefix,
  });

  @override
  State<KeyEaseFields> createState() => _KeyEaseFieldsState();
}

class _KeyEaseFieldsState extends State<KeyEaseFields> {
  late double _inSpeed;
  late double _outSpeed;

  /// Influences as the wells show them: per cent.
  late double _inInfluence;
  late double _outInfluence;

  /// Continuous: the speed out follows the speed in, so the curve runs
  /// through the key without a kink. On to begin with when the two already
  /// agree, which is what a joined pair of handles is.
  late bool _continuous;

  @override
  void initState() {
    super.initState();
    _adopt(widget.ease);
    _continuous = _both && (_inSpeed - _outSpeed).abs() < 1e-9;
  }

  @override
  void didUpdateWidget(KeyEaseFields old) {
    super.didUpdateWidget(old);
    if (old.ease != widget.ease) setState(() => _adopt(widget.ease));
  }

  void _adopt(KeyEase ease) {
    _inSpeed = ease.inSpeed ?? 0;
    _outSpeed = ease.outSpeed ?? 0;
    _inInfluence = (ease.inInfluence ?? 1 / 3) * 100;
    _outInfluence = (ease.outInfluence ?? 1 / 3) * 100;
  }

  bool get _both => widget.ease.hasIn && widget.ease.hasOut;

  void _setSpeed(bool isOut, double v) {
    setState(() {
      if (_continuous) {
        _inSpeed = v;
        _outSpeed = v;
      } else {
        _showSpeed(isOut, v);
      }
    });
    widget.onChanged(_continuous
        ? KeyEase(inSpeed: v, outSpeed: v)
        : isOut
            ? KeyEase(outSpeed: v)
            : KeyEase(inSpeed: v));
  }

  void _setInfluence(bool isOut, double percent) {
    setState(() => _showInfluence(isOut, percent));
    widget.onChanged(isOut
        ? KeyEase(outInfluence: percent / 100)
        : KeyEase(inInfluence: percent / 100));
  }

  /// A speed or influence moving in its well, before it is written.
  void _showSpeed(bool isOut, double v) {
    if (isOut) {
      _outSpeed = v;
    } else {
      _inSpeed = v;
    }
  }

  void _showInfluence(bool isOut, double percent) {
    if (isOut) {
      _outInfluence = percent;
    } else {
      _inInfluence = percent;
    }
  }

  /// Ticking Continuous gives the speed out the speed in, as the dialogue in
  /// After Effects does, and unticking changes nothing until a speed is typed.
  void _setContinuous(bool on) {
    final copy = on && (_inSpeed - _outSpeed).abs() >= 1e-9;
    setState(() {
      _continuous = on;
      if (copy) _outSpeed = _inSpeed;
    });
    if (copy) widget.onChanged(KeyEase(outSpeed: _inSpeed));
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final unit = l10n.unitPerSecond(widget.unit ?? '');
    final prefix = widget.keyPrefix;
    Widget speed(bool isOut) => keyFieldRow(
          t,
          isOut ? l10n.keySpeedOut : l10n.keySpeedIn,
          '$prefix-speed-${isOut ? 'out' : 'in'}',
          isOut ? _outSpeed : _inSpeed,
          min: -1e6,
          max: 1e6,
          decimals: 2,
          suffix: unit,
          labelWidth: _labelWidth,
          live: (v) => setState(() => _showSpeed(isOut, v)),
          set: (v) => _setSpeed(isOut, v),
        );
    Widget influence(bool isOut) => keyFieldRow(
          t,
          isOut ? l10n.keyInfluenceOut : l10n.keyInfluenceIn,
          '$prefix-influence-${isOut ? 'out' : 'in'}',
          isOut ? _outInfluence : _inInfluence,
          // Never quite nothing: an influence of zero is a handle with no
          // reach at all, which the evaluator has no span to divide by.
          min: 0.1,
          max: 100,
          decimals: 1,
          suffix: l10n.unitSymbolPercent,
          labelWidth: _labelWidth,
          live: (v) => setState(() => _showInfluence(isOut, v)),
          set: (v) => _setInfluence(isOut, v),
        );
    return Column(
      mainAxisSize: MainAxisSize.min,
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        if (widget.ease.hasIn) ...[speed(false), influence(false)],
        if (widget.ease.hasOut) ...[speed(true), influence(true)],
        if (_both)
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 4),
            child: Row(
              children: [
                HouseCheckbox(
                  key: ValueKey<String>('$prefix-continuous'),
                  value: _continuous,
                  onChanged: _setContinuous,
                ),
                const SizedBox(width: 6),
                Flexible(
                  child: Text(l10n.keyContinuous,
                      style: t.body,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis),
                ),
              ],
            ),
          ),
      ],
    );
  }
}

/// Room for "Influence out" in the body face.
const double _labelWidth = 78;
