// The per-layer right-click context menu, ported from the egui
// `layer_context_menu` (crates/lumit-ui/src/shell/timeline/menu.rs): the things
// you can do to a layer in one place (the house pattern — right-click, never
// scattered buttons). The wired entries (Duplicate, Delete, and the Solo /
// Enabled / Motion blur switch toggles) route to the real ops; the entries the
// Flutter frontend hasn't grown yet (Rename, Add effect/mask, Convert, Trim)
// route to `app.engine(...)` so a click is always answered honestly.

import 'package:flutter/widgets.dart';

import '../../bridge/bridge.dart';
import '../../state/app_state.dart';
import '../../widgets/controls.dart';

/// The actions the layer menu can raise.
enum _LayerMenuAction {
  rename,
  addEffect,
  addMask,
  duplicate,
  delete,
  solo,
  enabled,
  motionBlur,
  convert,
}

/// Show the layer context menu at [position] (global) and run the chosen action
/// against [app]. Mirrors the egui item set; unimplemented entries speak through
/// the status line.
Future<void> showLayerContextMenu({
  required BuildContext context,
  required AppStateStub app,
  required String compId,
  required BridgeLayer layer,
  required Offset position,
}) async {
  final action = await showLumitPopup<_LayerMenuAction>(
    context: context,
    position: position,
    builder: (close) => FloatSurface(
      width: 190,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          MenuRow(
            onPressed: () => close(_LayerMenuAction.rename),
            child: const Text('Rename'),
          ),
          MenuRow(
            onPressed: () => close(_LayerMenuAction.addEffect),
            child: const Text('Add effect'),
          ),
          MenuRow(
            onPressed: () => close(_LayerMenuAction.addMask),
            child: const Text('Add mask'),
          ),
          const _MenuDivider(),
          MenuRow(
            onPressed: () => close(_LayerMenuAction.duplicate),
            child: const _MenuLabelWithShortcut('Duplicate', 'Ctrl+D'),
          ),
          MenuRow(
            onPressed: () => close(_LayerMenuAction.delete),
            child: const Text('Delete'),
          ),
          const _MenuDivider(),
          MenuRow(
            selected: layer.switches.solo,
            onPressed: () => close(_LayerMenuAction.solo),
            child: const Text('Solo'),
          ),
          MenuRow(
            selected: layer.switches.visible,
            onPressed: () => close(_LayerMenuAction.enabled),
            child: const Text('Enabled'),
          ),
          MenuRow(
            selected: layer.switches.motionBlur,
            onPressed: () => close(_LayerMenuAction.motionBlur),
            child: const Text('Motion blur'),
          ),
          if (layer.kind == BridgeLayerKind.footage) ...[
            const _MenuDivider(),
            MenuRow(
              onPressed: () => close(_LayerMenuAction.convert),
              child: const Text('Convert to sequenced layer'),
            ),
          ],
        ],
      ),
    ),
  );
  if (action == null) return;
  switch (action) {
    case _LayerMenuAction.duplicate:
      app.duplicateLayer(compId, layer.id);
    case _LayerMenuAction.delete:
      app.deleteLayer(compId, layer.id);
    case _LayerMenuAction.solo:
      app.setLayerSwitch(compId, layer.id, 'solo', !layer.switches.solo);
    case _LayerMenuAction.enabled:
      app.setLayerSwitch(compId, layer.id, 'visible', !layer.switches.visible);
    case _LayerMenuAction.motionBlur:
      app.setLayerSwitch(
          compId, layer.id, 'motion_blur', !layer.switches.motionBlur);
    case _LayerMenuAction.rename:
      app.engine('Rename layer');
    case _LayerMenuAction.addEffect:
      app.engine('Add effect');
    case _LayerMenuAction.addMask:
      app.engine('Add mask');
    case _LayerMenuAction.convert:
      app.engine('Convert to sequenced layer');
  }
}

class _MenuDivider extends StatelessWidget {
  const _MenuDivider();
  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Padding(
      padding: const EdgeInsets.symmetric(vertical: 4, horizontal: 4),
      child: Container(height: 1, color: t.hairline),
    );
  }
}

class _MenuLabelWithShortcut extends StatelessWidget {
  final String label;
  final String shortcut;
  const _MenuLabelWithShortcut(this.label, this.shortcut);
  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Row(
      children: [
        Expanded(child: Text(label)),
        const SizedBox(width: 12),
        Text(shortcut, style: t.small.copyWith(color: t.textMuted)),
      ],
    );
  }
}
