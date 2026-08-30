// The two keyframe dialogues the Animation menu opens (K-244), on the K-444
// pattern: how a key is approached and left, and how far its handles reach.
//
// In plain terms: a keyframe has two sides. The one the animation arrives on
// and the one it leaves by, and each is either a *hold* (nothing moves until
// the next key), a straight line, a curve the user aims, or a curve the engine
// aims for them. **Interpolation** picks which of those four each side is;
// **Speed** sets how far the curved sides reach — the influence a tangent
// handle drags (K-505; a side's speed is what that handle carries, so it is
// not a fifth number to type).
//
// Neither dialogue writes anything. Each collects an answer and hands it back;
// the menu applies it, so the rule about *which* keys are affected lives in one
// place rather than in two dialogues that would drift.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';

/// What one side of a keyframe can be. The four the graph editor's own strip
/// offers, named the way After Effects names them.
enum KeyframeInterp { linear, bezier, hold, auto }

String keyframeInterpLabel(KeyframeInterp interp) => switch (interp) {
      KeyframeInterp.linear => l10n.keyInterpLinear,
      KeyframeInterp.bezier => l10n.keyInterpBezier,
      KeyframeInterp.hold => l10n.keyInterpHold,
      KeyframeInterp.auto => l10n.keyInterpAuto,
    };

/// Ask what the keys under the playhead should be approached and left by.
/// Completes with null when dismissed.
Future<({KeyframeInterp inSide, KeyframeInterp outSide})?>
    showKeyframeInterpolationFrb({
  required BuildContext context,
  required KeyframeInterp inSide,
  required KeyframeInterp outSide,
}) =>
        showLumitModal<({KeyframeInterp inSide, KeyframeInterp outSide})>(
          context: context,
          builder: (close) => _InterpBody(
            inSide: inSide,
            outSide: outSide,
            onConfirm: close,
            onCancel: () => close(null),
          ),
        );

class _InterpBody extends StatefulWidget {
  final KeyframeInterp inSide;
  final KeyframeInterp outSide;
  final ValueChanged<({KeyframeInterp inSide, KeyframeInterp outSide})>
      onConfirm;
  final VoidCallback onCancel;

  const _InterpBody({
    required this.inSide,
    required this.outSide,
    required this.onConfirm,
    required this.onCancel,
  });

  @override
  State<_InterpBody> createState() => _InterpBodyState();
}

class _InterpBodyState extends State<_InterpBody> {
  late KeyframeInterp _in = widget.inSide;
  late KeyframeInterp _out = widget.outSide;

  void _confirm() => widget.onConfirm((inSide: _in, outSide: _out));

  @override
  Widget build(BuildContext context) => _dialogue(
        context,
        title: l10n.menuKeyframeInterpolation,
        onConfirm: _confirm,
        onCancel: widget.onCancel,
        rows: [
          _labelled(
            context,
            l10n.keyInterpIn,
            BareDropdown<KeyframeInterp>(
              key: const ValueKey('key-interp-in'),
              value: _in,
              options: KeyframeInterp.values,
              label: keyframeInterpLabel,
              onChanged: (v) => setState(() => _in = v),
            ),
          ),
          _labelled(
            context,
            l10n.keyInterpOut,
            BareDropdown<KeyframeInterp>(
              key: const ValueKey('key-interp-out'),
              value: _out,
              options: KeyframeInterp.values,
              label: keyframeInterpLabel,
              onChanged: (v) => setState(() => _out = v),
            ),
          ),
        ],
      );
}

/// Ask how far each side's handle reaches, in per cent of the span beside it.
/// Completes with null when dismissed.
Future<({double inPercent, double outPercent})?> showKeyframeSpeedFrb({
  required BuildContext context,
  required double inPercent,
  required double outPercent,
}) =>
    showLumitModal<({double inPercent, double outPercent})>(
      context: context,
      builder: (close) => _SpeedBody(
        inPercent: inPercent,
        outPercent: outPercent,
        onConfirm: close,
        onCancel: () => close(null),
      ),
    );

class _SpeedBody extends StatefulWidget {
  final double inPercent;
  final double outPercent;
  final ValueChanged<({double inPercent, double outPercent})> onConfirm;
  final VoidCallback onCancel;

  const _SpeedBody({
    required this.inPercent,
    required this.outPercent,
    required this.onConfirm,
    required this.onCancel,
  });

  @override
  State<_SpeedBody> createState() => _SpeedBodyState();
}

class _SpeedBodyState extends State<_SpeedBody> {
  late double _in = widget.inPercent;
  late double _out = widget.outPercent;

  void _confirm() => widget.onConfirm((inPercent: _in, outPercent: _out));

  @override
  Widget build(BuildContext context) => _dialogue(
        context,
        title: l10n.menuKeyframeSpeed,
        onConfirm: _confirm,
        onCancel: widget.onCancel,
        rows: [
          _labelled(
            context,
            l10n.keyInfluenceIn,
            DragValueField(
              key: const ValueKey('key-influence-in'),
              value: _in,
              // Never quite nothing: an influence of zero is a handle with no
              // reach at all, which the evaluator has no span to divide by.
              min: 0.1,
              max: 100,
              decimals: 1,
              suffix: l10n.unitSymbolPercent,
              onChanged: (v) => setState(() => _in = v.toDouble()),
            ),
          ),
          _labelled(
            context,
            l10n.keyInfluenceOut,
            DragValueField(
              key: const ValueKey('key-influence-out'),
              value: _out,
              min: 0.1,
              max: 100,
              decimals: 1,
              suffix: l10n.unitSymbolPercent,
              onChanged: (v) => setState(() => _out = v.toDouble()),
            ),
          ),
        ],
      );
}

/// The shape both of these wear (K-444): a title, label-left rows in a 110px
/// column, and the single filled action with Cancel beside it. Enter applies
/// wherever the focus sits (K-243).
Widget _dialogue(
  BuildContext context, {
  required String title,
  required List<Widget> rows,
  required VoidCallback onConfirm,
  required VoidCallback onCancel,
}) {
  final t = ThemeScope.of(context).theme;
  return Focus(
    autofocus: true,
    onKeyEvent: (_, event) {
      if (event is! KeyDownEvent) return KeyEventResult.ignored;
      if (event.logicalKey != LogicalKeyboardKey.enter &&
          event.logicalKey != LogicalKeyboardKey.numpadEnter) {
        return KeyEventResult.ignored;
      }
      onConfirm();
      return KeyEventResult.handled;
    },
    child: FloatSurface(
      width: 320,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.only(bottom: 10),
            child: Text(title, style: t.bodyPrimary),
          ),
          for (var i = 0; i < rows.length; i++) ...[
            if (i > 0) const SizedBox(height: 8),
            rows[i],
          ],
          const SizedBox(height: 16),
          Row(
            mainAxisAlignment: MainAxisAlignment.end,
            children: [
              HouseButton(
                key: const ValueKey('keyframe-confirm'),
                primary: true,
                onPressed: onConfirm,
                child: Text(l10n.apply),
              ),
              const SizedBox(width: 8),
              HouseButton(
                key: const ValueKey('keyframe-cancel'),
                onPressed: onCancel,
                child: Text(l10n.cancel),
              ),
            ],
          ),
        ],
      ),
    ),
  );
}

Widget _labelled(BuildContext context, String label, Widget child) {
  final t = ThemeScope.of(context).theme;
  return Row(
    children: [
      SizedBox(width: 110, child: Text(label, style: t.small)),
      Expanded(child: child),
    ],
  );
}
