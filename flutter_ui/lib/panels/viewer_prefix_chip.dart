// The Viewer's **"at effect" chip** (K-528): the picture at one point in a
// layer's effect stack, in the Viewer rather than in a panel of its own.
//
// **In plain terms.** Select an effect — its heading in the Effect controls
// stack, or its box on the node graph, which are the same pick (K-300) — and
// this chip appears over the Viewer's picture saying *at* that effect's name.
// Click it and the Viewer shows the composition as it looks with that layer's
// effects stopping there: the blur applied and nothing after it. Click it again
// and the finished picture comes back. Nothing is soloed, bypassed or switched
// off, and the document is not touched.
//
// **Why it is a chip and not a panel.** It used to be one (K-448, K-486): a
// second, locked viewport showing a 256-pixel thumbnail. The owner's ruling is
// that a node preview is just the viewer — so the question is answered in the
// Viewer, at its own size and quality, through the one frame transport, and a
// whole second viewport went away.
//
// **What it costs.** The chip draws from the Dart-side read model and the
// selection, so a rebuild and a hover cost nothing at the seam. Toggling it
// costs exactly one render — the same render a playhead step costs — because
// the point rides the render request the Viewer was going to make anyway.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'effect_param_row_frb.dart' show effectLabelOf;
import 'viewer_panel_frb.dart' show viewerTagLeft, viewerTagTop;

/// The name of the point the chip would stop at, or `null` when there is no
/// single one — nothing picked, a whole run picked, or a pick whose effect this
/// layer no longer carries.
///
/// **The Source box is a point like any other** (N4): picked on the node
/// canvas, it names the layer's own picture before any effect, and the chip
/// says the layer's name.
///
/// Read from [LumitUiState.model], which is the frontend's own held copy of the
/// document, so this is a map lookup and never a call across the bridge.
String? prefixChipName(LumitUiState ui) {
  final point = ui.viewerPrefixPoint;
  if (point == null) return null;
  final info = ui.model.byId(point.$1.internallayerId)?.info;
  if (info == null) return null;
  final effect = point.$2;
  if (effect == null) return info.name;
  for (final on in info.effects) {
    if (on.id == effect) {
      // The user's own name where one is set (K-321), else the effect's label,
      // exactly as the Effect controls heading spells it.
      return on.customName ?? effectLabelOf(on.name);
    }
  }
  return null;
}

/// The chip itself. Returns a [Positioned], so it goes straight into the
/// Viewer's stage stack beside the selection tag — one line at the hookup, and
/// nothing of the Viewer's own layout to disturb.
class ViewerPrefixChip extends StatelessWidget {
  final LumitUiState uiState;
  const ViewerPrefixChip({super.key, required this.uiState});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // The three things that move without this panel being rebuilt: the pick,
    // the chip's own state, and the read model (an effect renamed or deleted
    // under it).
    return ListenableBuilder(
      listenable: Listenable.merge([
        uiState.selectedEffects,
        uiState.atSelectedEffect,
        // The Source box is picked on the canvas alone (N4), so the effect
        // selection is not what moves the chip onto it.
        uiState.graphNode,
        uiState.model,
      ]),
      builder: (context, _) {
        final name = prefixChipName(uiState);
        if (name == null) return const SizedBox.shrink();
        final on = uiState.atSelectedEffect.value;
        return Positioned(
          // Mirrors the selection tag across the picture: the tag says what is
          // selected, the chip says what is being looked at.
          right: viewerTagLeft,
          top: viewerTagTop,
          child: LumitTooltip(
            message: l10n.tipViewerAtEffect,
            child: GestureDetector(
              behavior: HitTestBehavior.opaque,
              onTap: () => uiState.setAtSelectedEffect(!on),
              child: Container(
                key: const ValueKey('viewer-at-effect'),
                constraints: const BoxConstraints(maxWidth: 220),
                padding: const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                decoration: BoxDecoration(
                  border: Border.all(color: on ? t.accent : t.textMuted),
                  borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                ),
                // Engaged, it takes the accent — the same way every other mark
                // over this picture says it is in force. The Viewer must never
                // quietly show an unfinished composition, and a chip that read
                // the same either way would be exactly that.
                child: Text(
                  l10n.viewerAtEffect(name),
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis,
                  style: t.small.copyWith(color: on ? t.accent : t.textMuted),
                ),
              ),
            ),
          ),
        );
      },
    );
  }
}
