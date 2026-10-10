// The tour Lumit ships: its steps, and the part of running it that touches
// the project.
//
// A first run opens on an empty project, where there is no Timeline and no
// picture to show anybody. So the tour makes a small composition to walk
// round, and takes it away again when it ends, leaving the project as it was
// found. A project with anything in it is never touched: the tour shows what
// is on screen and passes over the rest.
//
// Some of what it shows is not in the arrangement a first run opens on:
// Effect controls stands behind the Project panel, and the node graph is in
// the Nodes workspace. The tour brings those forward itself and puts the
// window back the way it found it when it ends.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/graph.dart';

import '../l10n/strings.dart';
import '../state/app_state.dart';
import '../state/dock.dart';
import '../state/keymap.dart';
import '../state/ui_state.dart';
import 'tour.dart';

/// The tour, in the order it is taken. The shortcuts are read from the keymap
/// and the settings as they stand, so a rebound key is the key the card names.
List<TourStep> tourSteps(LumitUiState ui) {
  final console = ui.keymap.chordFor('console.open');
  final palette = ui.keymap.chordFor('palette.open');
  final restore = _restorer(ui);
  // Both of these are about a layer's effects, so neither has anything to
  // show without a layer, or over a node graph composition, which has none.
  final effectControls = TourScene(
    offered: () =>
        _hasLayers(ui) && panelVisible(ui.split, Panel.effectControls),
    enter: () {
      _selectALayer(ui);
      ui.frontPanel(Panel.effectControls);
    },
    leave: restore,
  );
  final nodes = TourScene(
    offered: () => _hasLayers(ui),
    enter: () {
      _selectALayer(ui);
      ui.workspace.applyWorkspacePreset(WorkspacePreset.nodes);
    },
    leave: restore,
  );
  // The doors into the node search that Settings has left open.
  final interface = ui.workspace.interface;
  final searchKey = interface.tabOpensNodeSearch
      ? chordLabel('Tab')
      : interface.shiftAOpensNodeSearch
          ? chordLabel('Shift+A')
          : console;
  final searchBody = switch ((searchKey, interface.rightClickOpensNodeSearch)) {
    (final key?, true) => l10n.tourNodeSearchBody(key),
    (final key?, false) => l10n.tourNodeSearchBodyKey(key),
    (null, true) => l10n.tourNodeSearchBodyClick,
    (null, false) => null,
  };
  return [
    TourStep(
      target: 'dock-pane-${Panel.project.name}',
      title: Panel.project.title,
      body: l10n.tourProjectBody,
      side: TourSide.right,
    ),
    TourStep(
      target: 'viewer-comp-view',
      title: Panel.viewer.title,
      body: l10n.tourViewerBody,
      side: TourSide.below,
    ),
    TourStep(
      target: 'tool-bar-tools',
      title: l10n.tourToolsTitle,
      body: l10n.tourToolsBody,
      side: TourSide.below,
    ),
    TourStep(
      target: 'tl-outline-blocks',
      title: l10n.tourLayersTitle,
      body: l10n.tourLayersBody,
      side: TourSide.right,
    ),
    TourStep(
      target: 'tl-column-header',
      title: l10n.tourColumnsTitle,
      body: l10n.tourColumnsBody(l10n.columnSwitches, l10n.columnModes),
      side: TourSide.above,
      pointer: true,
    ),
    TourStep(
      target: 'tl-column-toggles',
      title: l10n.tourColumnTogglesTitle,
      body: l10n.tourColumnTogglesBody(
          l10n.columnSwitches, l10n.columnModes, l10n.columnParent),
      side: TourSide.above,
      pointer: true,
    ),
    // The stopwatch on the first layer's Position row, which is shut away
    // until the tour opens it: the same reveal the P key makes, asked for the
    // way the FX console asks. It stays open afterwards, as it would after
    // the key.
    TourStep(
      target: 'kf-stopwatch-tl-tf-position',
      prefix: true,
      opensFrom: 'tl-twirl-',
      open: () {
        final first = ui.model.layers.firstOrNull;
        if (first == null) return;
        ui.requestRevealProperty(
            first.layer.internallayerId, 'reveal.position');
      },
      title: l10n.tourPropertiesTitle,
      body: l10n.tourPropertiesBody,
      side: TourSide.right,
      pointer: true,
    ),
    TourStep(
      target: 'tl-mode-tabs',
      title: l10n.tourGraphTitle(
          l10n.timelineModeLayers, l10n.timelineModeGraph),
      body: l10n.tourGraphBody(l10n.timelineModeGraph),
      side: TourSide.above,
      pointer: true,
    ),
    TourStep(
      target: 'dock-pane-${Panel.effectsAndPresets.name}',
      title: Panel.effectsAndPresets.title,
      body: console == null
          ? l10n.tourEffectsBodyNoShortcut
          : l10n.tourEffectsBody(console),
      side: TourSide.left,
    ),
    TourStep(
      target: 'dock-pane-${Panel.effectControls.name}',
      scene: effectControls,
      title: Panel.effectControls.title,
      body: l10n.tourEffectControlsBody,
      side: TourSide.right,
    ),
    TourStep(
      target: 'fx-add',
      scene: effectControls,
      title: l10n.tourEffectMenusTitle,
      body: l10n.tourEffectMenusBody(l10n.addEffect),
      side: TourSide.right,
      pointer: true,
    ),
    TourStep(
      target: 'workspace-strip',
      title: l10n.tourWorkspacesTitle,
      body: palette == null
          ? l10n.tourWorkspacesBodyNoShortcut
          : l10n.tourWorkspacesBody(palette),
      side: TourSide.below,
      pointer: true,
    ),
    TourStep(
      target: 'dock-pane-${Panel.graph.name}',
      scene: nodes,
      title: Panel.graph.title,
      body: l10n.tourNodeGraphBody(WorkspacePreset.nodes.title),
      side: TourSide.right,
    ),
    // The search itself, put up over the middle of the canvas for the step
    // and taken down after it. It is the real console with the canvas's own
    // list, shown and not used, so nothing can be added by accident.
    if (searchBody != null)
      TourStep(
        target: 'fx-console-bar',
        opensFrom: 'graph-canvas',
        open: () => ui.nodeSearchExhibit.value = true,
        close: () => ui.nodeSearchExhibit.value = false,
        scene: nodes,
        title: l10n.tourNodeSearchTitle,
        body: searchBody,
        side: TourSide.right,
        pointer: true,
      ),
    TourStep(
      target: 'dock-pane-${Panel.node.name}',
      scene: nodes,
      title: Panel.node.title,
      body: l10n.tourNodePanelBody(Panel.effectControls.title),
      side: TourSide.left,
    ),
  ];
}

/// Whether the composition in front has layers to show the effects of.
bool _hasLayers(LumitUiState ui) =>
    !ui.model.isNodeGraph && ui.model.layers.isNotEmpty;

/// Select a layer when none is, since Effect controls and the node graph both
/// show the selected one: the first with an effect on it, or else the first.
/// It also picks that effect's box, so the Node panel has rows to show. A
/// selection the user made is left as it is.
void _selectALayer(LumitUiState ui) {
  if (ui.selectedLayer.value != null) return;
  final layers = ui.model.layers;
  if (layers.isEmpty) return;
  final entry = layers.firstWhere((l) => l.info.effects.isNotEmpty,
      orElse: () => layers.first);
  ui.setSelection([entry.layer]);
  final effect = entry.info.effects.firstOrNull;
  if (effect != null) ui.graphNode.value = BridgeNodeRef.effect(effect.id);
}

/// What puts the panels back the way they stand now: the arrangement, and
/// which workspace the strip has lit.
VoidCallback _restorer(LumitUiState ui) {
  final workspace = ui.workspace;
  final dock = workspace.dock.toJson();
  final preset = workspace.activePreset;
  final user = workspace.activeUserWorkspace;
  return () {
    final found = DockNode.fromJson(dock);
    if (found is! DockSplit) return;
    workspace.dock = found;
    workspace.activePreset = preset;
    workspace.activeUserWorkspace = user;
    workspace.touch();
  };
}

/// Raise the tour by itself, on a machine that has never finished or skipped
/// it. Does nothing on any later launch.
void maybeShowTourFrb(
    BuildContext context, LumitState state, LumitUiState ui) {
  if (ui.workspace.tourDone) return;
  showTourFrb(context, state, ui);
}

/// Raise the tour, whether or not it has been seen: Help ▸ Show the tour.
void showTourFrb(BuildContext context, LumitState state, LumitUiState ui) {
  final made = _demoItems = _makeDemo(state, ui);
  final up = showTour(
    context,
    ui.workspace,
    steps: tourSteps(ui),
    onEnd: () => _clearDemo(state, made),
  );
  if (!up) _clearDemo(state, made);
}

/// Give an empty, unsaved project a composition with two layers in it, one of
/// them wearing an effect, and say how many items the project then holds. Null when the project has
/// something of the user's in it already, and nothing was made.
int? _makeDemo(LumitState state, LumitUiState ui) {
  final project = state.project;
  if (project == null) return null;
  try {
    if (project.path() != null || project.getItems().isNotEmpty) return null;
    final comp = project.newComposition(name: l10n.tourDemoComp);
    comp.addNullLayer();
    comp.addTextLayer().addEffect(name: _demoEffect);
    ui.setSelectedComp(comp);
    state.notifyDocumentChanged();
    return project.getItems().length;
  } catch (_) {
    return null;
  }
}

/// The effect the tour's text layer wears, by its name in the catalogue: one
/// that shows on letters, for Effect controls and the node graph to list.
const String _demoEffect = 'glow';

/// Take the tour's composition away by starting the project afresh, which is
/// where it stood before. [made] is what [_makeDemo] answered. Left alone when
/// the project has gained anything since, such as files dropped onto the
/// Project panel while the tour was up: that is the user's, and it stays.
void _clearDemo(LumitState state, int? made) {
  final only = onlyTourDemo(state);
  _demoItems = null;
  // Nothing in it is the user's, so there is nothing to ask about saving.
  if (made != null && only) state.newProject(ask: false);
}

/// How many items the project held once the tour had made its composition,
/// while the tour is up over it.
int? _demoItems;

/// Whether the tour is up and everything in the project is its own, so
/// quitting part way has nothing of the user's to ask about saving.
bool onlyTourDemo(LumitState state) {
  final made = _demoItems;
  final project = state.project;
  if (made == null || project == null) return false;
  try {
    return project.path() == null && project.getItems().length == made;
  } catch (_) {
    return false;
  }
}
