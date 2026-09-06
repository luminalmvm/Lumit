// The Easing panel (docs/07 §5.4): the easing editor with somewhere to live.
//
// In plain terms: the same unit box the Easing… button used to open as a popup,
// docked instead. The difference is not how it looks — it is the same widget —
// but how long it lasts. A popup closes on any click outside it, and changing
// which keyframes are selected *is* a click outside, so one shape could only
// ever be tried on one selection. Docked, the shape stays put: pick some keys,
// Apply, pick some more, Apply again.
//
// The panel does not own the selection and never asks what it is. The Timeline
// publishes a callback while it can take a shape (`LumitUiState.easingApply`,
// the same claim idiom Delete and Copy use), and this panel
// presses it. Null means nothing is listening — no Timeline on screen, or its
// graph is in the speed lens, where a shape drawn against value travel does not
// belong — and Apply greys with a line saying so.
//
// Under the editor, while exactly one keyframe is selected, the panel shows
// that key's speed and influence on each side as wells (`LumitUiState.easingKey`,
// the same claim idiom again). This is the one thing it learns about the
// selection, and it is the key's numbers rather than the selection itself: the
// editor above still never knows what a shape it sends will land on.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'easing_curve.dart';
import 'easing_editor.dart';
import 'key_ease_fields.dart';

class EasingPanelFrb extends StatelessWidget {
  const EasingPanelFrb({super.key});

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    return SingleChildScrollView(
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // Listening to the claim alone, not to the whole shell: the editor
          // holds the shape being drawn in its own State, and rebuilding it on
          // every unrelated notification is how a half-dragged handle would
          // jump back.
          ValueListenableBuilder<ValueChanged<EasingCurve>?>(
            valueListenable: ui.easingApply,
            builder: (context, apply, _) => EasingEditor(
              // One State for the panel's whole life, so the drawn shape
              // survives the claim coming and going. Without the key, Flutter
              // would still reuse it - same type, same position - but saying
              // so is cheaper than relying on it.
              key: const ValueKey('easing-panel-editor'),
              onApply: apply,
              whyNot: l10n.easingNeedsTheValueLens,
            ),
          ),
          // The selected key's own numbers, its own listener for the same
          // reason: a key changing under the panel must not rebuild the box.
          ValueListenableBuilder<KeyEaseClaim?>(
            valueListenable: ui.easingKey,
            builder: (context, claim, _) => claim == null
                ? const SizedBox.shrink()
                : _SelectedKey(claim: claim),
          ),
        ],
      ),
    );
  }
}

/// The foot of the panel while one key is selected: which frame it sits on,
/// then its speed and influence on each side, editable.
///
/// Keyed by the key it shows, so moving the selection to another key gives
/// the wells a fresh State rather than one still holding the last key's
/// half-typed number, while the same key with new numbers keeps its State and takes
/// them (see [KeyEaseFields]).
class _SelectedKey extends StatelessWidget {
  final KeyEaseClaim claim;
  const _SelectedKey({required this.claim});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Padding(
      key: const ValueKey('easing-key'),
      padding: const EdgeInsets.fromLTRB(10, 0, 10, 10),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 6),
            child: Row(
              children: [
                Text(l10n.graphKeyKicker.toUpperCase(), style: t.kicker),
                const SizedBox(width: 8),
                Text(l10n.graphKeyFrame(claim.frame),
                    style: t.mono.copyWith(fontSize: 10, color: t.textPrimary)),
              ],
            ),
          ),
          KeyEaseFields(
            key: ValueKey<String>(
                'easing-key-${claim.channelId}#${claim.index}'),
            ease: claim.ease,
            unit: claim.unit,
            keyPrefix: 'easing-key',
            onChanged: claim.write,
          ),
        ],
      ),
    );
  }
}
