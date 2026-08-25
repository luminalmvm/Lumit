// The Easing panel (docs/07 §5.4, K-349): the easing editor with somewhere to live.
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
// the same claim idiom Delete and Copy use — K-234, K-300), and this panel
// presses it. Null means nothing is listening — no Timeline on screen, or its
// graph is in the speed lens, where a shape drawn against value travel does not
// belong — and Apply greys with a line saying so.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import 'easing_curve.dart';
import 'easing_editor.dart';

class EasingPanelFrb extends StatelessWidget {
  const EasingPanelFrb({super.key});

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    // Listening to the claim alone, not to the whole shell: the editor holds
    // the shape being drawn in its own State, and rebuilding it on every
    // unrelated notification is how a half-dragged handle would jump back.
    return ValueListenableBuilder<ValueChanged<EasingCurve>?>(
      valueListenable: ui.easingApply,
      builder: (context, apply, _) => SingleChildScrollView(
        child: EasingEditor(
          // One State for the panel's whole life, so the drawn shape survives
          // the claim coming and going. Without the key, Flutter would still
          // reuse it — same type, same position — but saying so is cheaper than
          // relying on it.
          key: const ValueKey('easing-panel-editor'),
          onApply: apply,
          whyNot: l10n.easingNeedsTheValueLens,
        ),
      ),
    );
  }
}
