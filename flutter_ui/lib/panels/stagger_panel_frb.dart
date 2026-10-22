// The Stagger panel: start the selected layers one after another, a set
// number of frames apart, or line them up on one frame again.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../shell/menu_layer_frb.dart' show selectedEntries;
import '../state/dock.dart';
import '../state/workspace.dart';
import '../widgets/controls.dart';
import 'stagger.dart';
import 'text_target.dart' show textCell;

/// Put the Stagger panel up and in front: what Layer ▸ Stagger layers… and
/// the row menu do.
void showStaggerPanel(LumitUiState ui) {
  setPanelVisible(ui.split, Panel.stagger, true);
  activatePanelTab(ui.split, Panel.stagger);
  ui.activePane.value = Panel.stagger.pane();
  ui.workspace.touch();
}

String staggerOrderLabel(LayerStaggerOrder order) => switch (order) {
      LayerStaggerOrder.topDown => l10n.easeStaggerTopDown,
      LayerStaggerOrder.bottomUp => l10n.easeStaggerBottomUp,
      LayerStaggerOrder.clicked => l10n.staggerClicked,
      LayerStaggerOrder.random => l10n.staggerRandom,
    };

class StaggerPanelFrb extends StatefulWidget {
  const StaggerPanelFrb({super.key});

  @override
  State<StaggerPanelFrb> createState() => _StaggerPanelFrbState();
}

class _StaggerPanelFrbState extends State<StaggerPanelFrb> {
  late final LumitUiState _ui =
      Provider.of<LumitUiState>(context, listen: false);

  /// The step while its well is being dragged, kept off the workspace store
  /// until the drag ends.
  int? _live;

  void _apply(int step) {
    final comp = _ui.selectedComp;
    final layers = [
      for (final entry in selectedEntries(_ui))
        if (!entry.info.switches.locked) entry.layer.internallayerId,
    ];
    if (comp == null || layers.length < 2) return;
    try {
      comp.staggerLayers(
        layerIds: inStaggerOrder(layers, _ui.workspace.staggerOrder,
            clicked: _ui.selectionInClickOrder, layerOf: (id) => id),
        step: step,
      );
    } catch (_) {
      return;
    }
    Provider.of<LumitState>(context, listen: false).notifyDocumentChanged();
  }

  void _setStep(num v) {
    setState(() => _live = null);
    _ui.workspace.setStagger(step: v.round());
  }

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: Listenable.merge([_ui.model, _ui.selectedLayers]),
      builder: (context, _) {
        final t = ThemeScope.of(context).theme;
        final workspace = _ui.workspace;
        final live =
            selectedEntries(_ui).where((e) => !e.info.switches.locked).length >
                1;
        return Container(
          color: t.surface0,
          child: ListView(
            padding: const EdgeInsets.only(top: 6),
            children: [
              textCell(
                t,
                l10n.staggerOffset,
                Row(
                  children: [
                    SizedBox(
                      width: 64,
                      child: DragValueField(
                        key: const ValueKey('stagger-step'),
                        value: _live ?? workspace.staggerStep,
                        min: 0,
                        max: 100000,
                        decimals: 0,
                        resetTo: 1,
                        onChangeLive: (v) => setState(() => _live = v.round()),
                        onChangeEnd: _setStep,
                        onDragCancel: () => setState(() => _live = null),
                        onChanged: _setStep,
                      ),
                    ),
                    const SizedBox(width: 6),
                    Text(l10n.unitSymbolFrames,
                        style: t.body.copyWith(color: t.textMuted)),
                  ],
                ),
              ),
              textCell(
                t,
                l10n.staggerOrder,
                SizedBox(
                  width: 120,
                  child: BareDropdown<LayerStaggerOrder>(
                    key: const ValueKey('stagger-order'),
                    dense: true,
                    value: workspace.staggerOrder,
                    options: LayerStaggerOrder.values,
                    label: staggerOrderLabel,
                    onChanged: (order) =>
                        setState(() => workspace.setStagger(order: order)),
                  ),
                ),
              ),
              Padding(
                padding: const EdgeInsets.fromLTRB(10, 8, 10, 8),
                child: Row(
                  children: [
                    HouseButton(
                      key: const ValueKey('stagger-line-up'),
                      onPressed: live ? () => _apply(0) : null,
                      child: Text(l10n.staggerLineUp),
                    ),
                    const SizedBox(width: 8),
                    HouseButton(
                      key: const ValueKey('stagger-apply'),
                      primary: true,
                      onPressed:
                          live ? () => _apply(workspace.staggerStep) : null,
                      child: Text(l10n.staggerApply),
                    ),
                  ],
                ),
              ),
            ],
          ),
        );
      },
    );
  }
}
