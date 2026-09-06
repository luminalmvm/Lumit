// The two keyframe dialogues the Animation menu opens, on the shared dialogue
// pattern: how a key is approached and left, and how far its handles reach.
//
// In plain terms: a keyframe has two sides. The one the animation arrives on
// and the one it leaves by, and each is either a *hold* (nothing moves until
// the next key), a straight line, a curve the user aims, or a curve the engine
// aims for them. **Interpolation** picks which of those four each side is;
// **Speed** sets the two numbers each curved side carries, how fast the curve
// runs through the key and how far its handle reaches, as exact figures
// rather than by dragging the handle (`panels/key_ease_fields.dart`).
//
// Neither dialogue writes anything. Each collects an answer and hands it back;
// the menu applies it, so the rule about *which* keys are affected lives in one
// place rather than in two dialogues that would drift.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import '../l10n/strings.dart';
import '../panels/graph_maths.dart' show KeyEase;
import '../panels/key_ease_fields.dart';
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

/// Ask for a key's speed and influence on each side it has. Opens on [ease],
/// whose sides with no numbers are not offered - an end key, or the one dot
/// the speed graph's menu was opened on. [unit] is what the speed is per
/// second of.
///
/// Completes with the numbers that were changed, and only those, or null when
/// dismissed. Nothing changed comes back as an empty [KeyEase].
Future<KeyEase?> showKeyframeSpeedFrb({
  required BuildContext context,
  required KeyEase ease,
  required String? unit,
}) =>
    showLumitModal<KeyEase>(
      context: context,
      builder: (close) => _SpeedBody(
        ease: ease,
        unit: unit,
        onConfirm: close,
        onCancel: () => close(null),
      ),
    );

class _SpeedBody extends StatefulWidget {
  final KeyEase ease;
  final String? unit;
  final ValueChanged<KeyEase> onConfirm;
  final VoidCallback onCancel;

  const _SpeedBody({
    required this.ease,
    required this.unit,
    required this.onConfirm,
    required this.onCancel,
  });

  @override
  State<_SpeedBody> createState() => _SpeedBodyState();
}

class _SpeedBodyState extends State<_SpeedBody> {
  /// What has been typed so far, each edit laid over the last, so Apply hands
  /// back one answer however many wells were visited.
  KeyEase _edit = const KeyEase();

  @override
  Widget build(BuildContext context) => _dialogue(
        context,
        title: l10n.menuKeyframeSpeed,
        onConfirm: () => widget.onConfirm(_edit),
        onCancel: widget.onCancel,
        rows: [
          KeyEaseFields(
            ease: widget.ease,
            unit: widget.unit,
            keyPrefix: 'key',
            onChanged: (edit) => _edit = _edit.merge(edit),
          ),
        ],
      );
}

/// The shape both of these wear: a title, label-left rows in a 110px column,
/// and the single filled action with Cancel beside it. Enter applies wherever
/// the focus sits.
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
