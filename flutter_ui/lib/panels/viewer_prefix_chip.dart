// The Viewer's **"at effect" chip**: the picture at one point in a
// layer's effect stack, in the Viewer rather than in a panel of its own.
//
// **In plain terms.** Select an effect — its heading in the Effect controls
// stack, or its box on the node graph, which are the same pick — and
// this chip appears over the Viewer's picture saying *at* that effect's name.
// Click it and the Viewer shows the composition as it looks with that layer's
// effects stopping there: the blur applied and nothing after it. Click it again
// and the finished picture comes back. Nothing is soloed, bypassed or switched
// off, and the document is not touched.
//
// **Why it is a chip and not a panel.** It used to be one: a
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
import 'package:lumit_flutter/src/rust/api/graph.dart' show BridgeNodeRef;

import '../l10n/strings.dart';
import '../widgets/controls.dart';
import 'effect_param_row_frb.dart' show effectLabelOf;
import 'timeline_extras_frb.dart' show showMenuAt;
import 'viewer_menus.dart' show viewerRow;
import 'viewer_panel_frb.dart'
    show viewerChipPlate, viewerTagLeft, viewerTagMaxWidth, viewerTagTop;

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
  // A box inside a Custom shader names itself by the word its canvas handed
  // over.
  if (ui.viewerShaderPoint case final shader?) {
    return ui.shaderGraphWords[shader.node];
  }
  // A node graph's box names itself (docs/impl/node-graph-comp.md §4.5). The
  // label rode in with the pick, so this is a field read; a driver, a value
  // Input and the Output make no picture and name no point.
  if (ui.viewerGraphPoint != null) return ui.compGraphNode.value?.label;
  final point = ui.viewerPrefixPoint;
  if (point == null) return null;
  final info = ui.model.byId(point.$1.internallayerId)?.info;
  if (info == null) return null;
  final effect = point.$2;
  if (effect == null) return info.name;
  for (final on in info.effects) {
    if (on.id == effect) {
      // The user's own name where one is set, else the effect's label,
      // exactly as the Effect controls heading spells it.
      return on.customName ?? effectLabelOf(on.name);
    }
  }
  return null;
}

/// Whether stopping at the picked point would show anything other than the
/// finished picture: some enabled effect that draws comes after it. A box on a
/// node graph or in a shader always counts, since the graph decides that.
bool prefixChipDiffers(LumitUiState ui) {
  final point = ui.viewerPrefixPoint;
  if (point == null || ui.viewerShaderPoint != null) return true;
  if (ui.viewerGraphPoint != null) return true;
  final effects = ui.model.byId(point.$1.internallayerId)?.info.effects;
  if (effects == null) return true;
  // The Source box stops before every effect.
  final at = point.$2 == null
      ? -1
      : effects.indexWhere((on) => on.id == point.$2);
  return effects.skip(at + 1).any((on) => on.enabled && !on.audio);
}

/// The chip itself. Returns a [Positioned], so it goes straight into the
/// Viewer's stage stack beside the selection tag — one line at the hookup, and
/// nothing of the Viewer's own layout to disturb.
class ViewerPrefixChip extends StatelessWidget {
  final LumitUiState uiState;

  /// How far the rulers come down from the stage's top.
  final double top;

  const ViewerPrefixChip({super.key, required this.uiState, this.top = 0});

  void _menu(BuildContext context, Offset at, bool on) {
    final point = uiState.viewerPrefixPoint;
    final effect = point?.$2;
    showMenuAt<void>(
      context: context,
      position: at,
      rows: (close) => [
        viewerRow(l10n.graphViewHere, () => uiState.setAtSelectedEffect(!on),
            () => close(null),
            id: 'viewer-at-view-here', ticked: on),
        // A layer's point is a box on its graph; a node graph's box is
        // already on the canvas it was picked on.
        if (point != null &&
            uiState.viewerGraphPoint == null &&
            uiState.viewerShaderPoint == null)
          viewerRow(
              l10n.fxMenuRevealGraph,
              () => uiState.revealBox(
                  point.$1,
                  effect == null
                      ? const BridgeNodeRef.source()
                      : BridgeNodeRef.effect(effect)),
              () => close(null),
              id: 'viewer-at-reveal'),
      ],
    );
  }

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
        // And a node graph's box is picked on its own canvas, which is a third
        // surface with a notifier of its own.
        uiState.compGraphNode,
        // And a box inside a Custom shader, on the shader's own canvas.
        uiState.shaderGraphNode,
        uiState.model,
      ]),
      builder: (context, _) {
        final name = prefixChipName(uiState);
        final on = uiState.atSelectedEffect.value;
        // Off and stopping where the picture already ends, it would change
        // nothing, so it stays out of the way.
        if (name == null || (!on && !prefixChipDiffers(uiState))) {
          return const SizedBox.shrink();
        }
        return Positioned(
          // Mirrors the selection tag across the picture: the tag says what is
          // selected, the chip says what is being looked at.
          right: viewerTagLeft,
          top: viewerTagTop + top,
          child: LumitTooltip(
            message: l10n.tipViewerAtEffect,
            child: HoverMark(
              swells: true,
              builder: (context, lit) {
                final ink = on
                    ? t.accent
                    : Color.lerp(t.textMuted, t.textPrimary, lit)!;
                return GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  onTap: () => uiState.setAtSelectedEffect(!on),
                  // Its own menu, and the press stops here: under the chip is
                  // the picture, whose menu would pick the layer beneath.
                  onSecondaryTapUp: (d) => _menu(context, d.globalPosition, on),
                  child: Container(
                    key: const ValueKey('viewer-at-effect'),
                    constraints:
                        const BoxConstraints(maxWidth: viewerTagMaxWidth),
                    padding:
                        const EdgeInsets.symmetric(horizontal: 6, vertical: 2),
                    // The selection tag's plate, so the two read as a pair.
                    decoration: BoxDecoration(
                      color: t.surface0.withValues(alpha: viewerChipPlate),
                      borderRadius:
                          BorderRadius.circular(t.tokens.controlRadius),
                    ),
                    // Engaged, it takes the accent — the same way every other
                    // mark over this picture says it is in force. The Viewer
                    // must never quietly show an unfinished composition, and a
                    // chip that read the same either way would be exactly that.
                    child: Text(
                      l10n.viewerAtEffect(name),
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: t.small.copyWith(color: ink),
                    ),
                  ),
                );
              },
            ),
          ),
        );
      },
    );
  }
}
