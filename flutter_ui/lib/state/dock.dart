// The dock layout model, ported from crates/lumit-ui/src/shell/dock.rs
// (which leans on egui_tiles). A serialisable tree of splits, tab groups and
// panes; the widget layer renders it and the model owns the invariants.
//
// In plain terms: the workspace is a tree. A *split* lays its children side by
// side (or stacked) with weighted shares; a *tabs* node stacks panels behind
// one another with a tab bar; a *pane* is one panel. A pane that sits alone —
// not inside a tabs node — renders bare, with no tab bar (K-086).

import 'package:lumit_flutter/l10n/strings.dart';

/// The dockable panels — glossary names (docs/01-GLOSSARY.md §7).

enum Panel {
  project,
  viewer,
  timeline,
  effectControls,
  effectsAndPresets,
  scopes,
  debug,
  hierarchy,
  easing,

  /// The layer's effect stack drawn as nodes and wires, plus the drivers wired
  /// into its parameters (K-471). A second *view* of the same document, not a
  /// second document.
  graph;

  String get title => switch (this) {
        Panel.project => l10n.panelProject,
        Panel.viewer => l10n.panelViewer,
        Panel.timeline => l10n.panelTimeline,
        Panel.effectControls => l10n.effectControls,
        Panel.effectsAndPresets => l10n.panelEffectsAndPresets,
        Panel.scopes => l10n.panelScopes,
        Panel.hierarchy => l10n.panelHierarchy,
        Panel.easing => l10n.panelEasing,
        Panel.graph => l10n.panelGraph,
        Panel.debug => l10n.panelDebug
      };
}

enum DockAxis { horizontal, vertical }

sealed class DockNode {
  Map<String, dynamic> toJson();

  static DockNode fromJson(Map<String, dynamic> j) => switch (j['kind']) {
        'pane' => DockPane(Panel.values.asNameMap()[j['panel']]!),
        'tabs' => DockTabs(
            [
              for (final c in j['children'] as List)
                fromJson(c as Map<String, dynamic>) as DockPane
            ],
            active: j['active'] as int? ?? 0,
          ),
        'split' => DockSplit(
            j['axis'] == 'vertical' ? DockAxis.vertical : DockAxis.horizontal,
            [
              for (final c in j['children'] as List)
                fromJson(c as Map<String, dynamic>)
            ],
            [for (final s in j['shares'] as List) (s as num).toDouble()],
          ),
        _ => throw FormatException('unknown dock node: ${j['kind']}'),
      };
}

class DockPane extends DockNode {
  final Panel panel;
  DockPane(this.panel);

  @override
  Map<String, dynamic> toJson() => {'kind': 'pane', 'panel': panel.name};
}

/// A tab group. Children are panes (egui_tiles allows nesting, but the
/// shipped frontend only ever tabs panes — the port models what ships).
class DockTabs extends DockNode {
  final List<DockPane> children;
  int active;
  DockTabs(this.children, {this.active = 0});

  DockPane get activePane => children[active.clamp(0, children.length - 1)];

  @override
  Map<String, dynamic> toJson() => {
        'kind': 'tabs',
        'active': active,
        'children': [for (final c in children) c.toJson()],
      };
}

class DockSplit extends DockNode {
  final DockAxis axis;
  final List<DockNode> children;

  /// Weighted shares, same length as children, normalised on use.
  final List<double> shares;

  DockSplit(this.axis, this.children, this.shares)
      : assert(children.length == shares.length);

  @override
  Map<String, dynamic> toJson() => {
        'kind': 'split',
        'axis': axis.name,
        // A copy: the live list is mutated in place as splitters are dragged,
        // and a caller that keeps this map — the per-project session (K-245)
        // does — would otherwise be holding the layout rather than a record of
        // what it was.
        'shares': [...shares],
        'children': [for (final c in children) c.toJson()],
      };
}

/// The default workspace (docs/07 §1.6 "Edit"): a vertical root (upper band
/// 0.68, Timeline 0.32 across the full width); the upper band horizontal
/// (left tab group 0.22, Viewer 0.58, right tab group 0.20). The left group
/// tabs Project (fronted), Effect controls, Hierarchy; the right group tabs
/// Effects & presets (fronted), Scopes, Debug — the spec's right-hand
/// Effects & presets column, which this layout used to bury as a left tab
/// behind Project while fronting Debug on the right. Viewer and Timeline sit
/// alone and render bare.
DockSplit defaultLayout() => DockSplit(
      DockAxis.vertical,
      [
        DockSplit(
          DockAxis.horizontal,
          [
            DockTabs([
              DockPane(Panel.project),
              DockPane(Panel.effectControls),
              DockPane(Panel.hierarchy),
            ]),
            DockPane(Panel.viewer),
            DockTabs([
              DockPane(Panel.effectsAndPresets),
              DockPane(Panel.scopes),
              DockPane(Panel.debug),
            ]),
          ],
          [0.22, 0.58, 0.20],
        ),
        DockPane(Panel.timeline),
      ],
      [0.68, 0.32],
    );

/// The shipped workspace presets (docs/07 §1.6): much the same panel
/// inventory, arranged for different work. Structure only, per the spec; the
/// Audio preset stands in with a taller Timeline (whose waveform lanes are the
/// v1 audio surface) until the Audio panel itself is built.
///
/// Retiming is the one preset that changes the inventory rather than only the
/// arrangement (K-349): the Easing panel is not in the others, because a panel
/// nobody asked for should not appear in an arrangement they already know.
enum WorkspacePreset {
  edit,
  effects,
  colour,
  audio,
  retiming;

  String get title => switch (this) {
        WorkspacePreset.edit => l10n.workspaceEdit,
        WorkspacePreset.effects => l10n.workspaceEffects,
        WorkspacePreset.colour => l10n.workspaceColour,
        WorkspacePreset.audio => l10n.workspaceAudio,
        WorkspacePreset.retiming => l10n.workspaceRetiming,
      };
}

/// The factory layout of one preset — restorable at any time, never touched
/// by the user's own rearranging (that persists separately).
DockSplit presetLayout(WorkspacePreset preset) => switch (preset) {
      // Edit is the default arrangement.
      WorkspacePreset.edit => defaultLayout(),
      // Effect controls promoted to its own column beside the Project panel;
      // Effects & presets expanded right with Scopes tabbed behind; the
      // Timeline slightly shorter than Edit.
      WorkspacePreset.effects => DockSplit(
          DockAxis.vertical,
          [
            DockSplit(
              DockAxis.horizontal,
              [
                DockTabs([
                  DockPane(Panel.project),
                  DockPane(Panel.hierarchy),
                ]),
                DockPane(Panel.effectControls),
                DockPane(Panel.viewer),
                DockTabs([
                  DockPane(Panel.effectsAndPresets),
                  DockPane(Panel.scopes),
                  DockPane(Panel.debug),
                ]),
              ],
              [0.16, 0.20, 0.44, 0.20],
            ),
            DockPane(Panel.timeline),
          ],
          [0.72, 0.28],
        ),
      // Scopes given a wide right-hand column; Effect controls left;
      // Effects & presets tabbed away; Viewer centre-dominant.
      WorkspacePreset.colour => DockSplit(
          DockAxis.vertical,
          [
            DockSplit(
              DockAxis.horizontal,
              [
                DockTabs([
                  DockPane(Panel.effectControls),
                  DockPane(Panel.project),
                  DockPane(Panel.effectsAndPresets),
                  DockPane(Panel.hierarchy),
                ]),
                DockPane(Panel.viewer),
                DockTabs([
                  DockPane(Panel.scopes),
                  DockPane(Panel.debug),
                ]),
              ],
              [0.18, 0.52, 0.30],
            ),
            DockPane(Panel.timeline),
          ],
          [0.72, 0.28],
        ),
      // The Timeline taller than Edit with its waveform lanes; the Viewer
      // reduced. The Audio panel joins this arrangement when it is built.
      WorkspacePreset.audio => DockSplit(
          DockAxis.vertical,
          [
            DockSplit(
              DockAxis.horizontal,
              [
                DockTabs([
                  DockPane(Panel.project),
                  DockPane(Panel.effectControls),
                  DockPane(Panel.effectsAndPresets),
                  DockPane(Panel.hierarchy),
                ]),
                DockPane(Panel.viewer),
                DockTabs([
                  DockPane(Panel.scopes),
                  DockPane(Panel.debug),
                ]),
              ],
              [0.24, 0.56, 0.20],
            ),
            DockPane(Panel.timeline),
          ],
          [0.55, 0.45],
        ),
      // Retiming (K-349): the arrangement for shaping movement. The **Easing**
      // panel takes the right-hand column outright rather than tabbing behind
      // Scopes — the whole point of the panel over the popup is that it stays
      // on screen while the selection changes underneath it, and a panel behind
      // a tab is a panel you have to keep fetching. The Timeline is as tall as
      // Audio's, because retiming is timeline work: the graph editor is where
      // the eye is, and the shape is drawn beside it.
      WorkspacePreset.retiming => DockSplit(
          DockAxis.vertical,
          [
            DockSplit(
              DockAxis.horizontal,
              [
                DockTabs([
                  DockPane(Panel.project),
                  DockPane(Panel.effectControls),
                  DockPane(Panel.effectsAndPresets),
                  DockPane(Panel.hierarchy),
                ]),
                DockPane(Panel.viewer),
                DockPane(Panel.easing),
              ],
              [0.20, 0.58, 0.22],
            ),
            DockPane(Panel.timeline),
          ],
          [0.55, 0.45],
        ),
    };

/// Every panel present in the tree, in visit order.
List<Panel> panelsIn(DockNode node) => switch (node) {
      DockPane(:final panel) => [panel],
      DockTabs(:final children) => [for (final c in children) c.panel],
      DockSplit(:final children) => [
          for (final c in children) ...panelsIn(c),
        ],
    };

/// Whether `panel` is anywhere in the tree — which is what "visible" means for
/// a dock: a panel that is not in the arrangement is not on screen, and one
/// that is can always be brought to the front of its group.
bool panelVisible(DockNode node, Panel panel) => panelsIn(node).contains(panel);

/// Add or drop `panel`, for the Window menu's tick list. A no-op when the tree
/// already agrees with `visible`.
///
/// Showing stacks it into the first tab group, fronted — a panel you just asked
/// for is the one you want to look at. With no tab group at all it pairs up
/// with the first tile instead, so it never has to invent a share of the
/// window. Hiding drops the pane and simplifies, exactly as closing a tab does.
/// The last panel standing cannot be hidden: an empty dock has no way back.
void setPanelVisible(DockSplit root, Panel panel, bool visible) {
  if (panelsIn(root).contains(panel) == visible) return;
  if (!visible) {
    if (panelsIn(root).length <= 1) return;
    _removePanel(root, panel);
    simplify(root);
    return;
  }
  final tabs = _firstTabs(root);
  if (tabs != null) {
    tabs.children.add(DockPane(panel));
    tabs.active = tabs.children.length - 1;
    return;
  }
  final first = root.children.first;
  if (first is DockPane) {
    root.children[0] = DockTabs([first, DockPane(panel)], active: 1);
    return;
  }
  root.children.insert(0, DockPane(panel));
  root.shares.insert(0, 0.2);
}

/// The first tab group in visit order, or null when every panel sits alone.
DockTabs? _firstTabs(DockNode node) {
  switch (node) {
    case DockPane():
      return null;
    case DockTabs():
      return node;
    case DockSplit(:final children):
      for (final child in children) {
        final found = _firstTabs(child);
        if (found != null) return found;
      }
      return null;
  }
}

/// Bring `panel`'s tab to the front of whichever tab group holds it (the
/// start-up "always open on Project" rule).
void activatePanelTab(DockNode node, Panel panel) {
  switch (node) {
    case DockPane():
      break;
    case DockTabs(:final children):
      final i = children.indexWhere((c) => c.panel == panel);
      if (i >= 0) node.active = i;
    case DockSplit(:final children):
      for (final c in children) {
        activatePanelTab(c, panel);
      }
  }
}

// --- Re-dock operations (dock.rs drag-to-redock, via egui_tiles) ----------

/// Where a dragged panel lands relative to the target pane: stacked into its
/// tab group, or splitting off one of its four sides.
enum DropPosition { left, right, above, below, stack }

/// Move `dragged` so it lands relative to `target`'s pane per `pos`, then
/// simplify (dock.rs::dock_simplification_options). A panel dropped onto
/// itself is a no-op. After the move every panel still appears exactly once,
/// each shares list matches its children length, and all shares are positive.
void movePanel(
  DockSplit root,
  Panel dragged,
  Panel target,
  DropPosition pos,
) {
  if (dragged == target) return;
  final present = panelsIn(root).toSet();
  if (!present.contains(dragged) || !present.contains(target)) return;

  _removePanel(root, dragged);
  final loc = _tileOf(root, target);
  // The target should always survive the removal; bail defensively if not.
  if (loc == null) return;

  final draggedPane = DockPane(dragged);
  if (pos == DropPosition.stack) {
    final tile = loc.tile;
    if (tile is DockTabs) {
      tile.children.add(draggedPane);
      tile.active = tile.children.length - 1;
    } else {
      // A solo pane becomes a two-tab group, the newcomer fronted.
      loc.split.children[loc.index] =
          DockTabs([tile as DockPane, draggedPane], active: 1);
    }
  } else {
    final axis = (pos == DropPosition.left || pos == DropPosition.right)
        ? DockAxis.horizontal
        : DockAxis.vertical;
    final before = pos == DropPosition.left || pos == DropPosition.above;
    final split = loc.split;
    if (split.axis == axis) {
      // Same axis: sit adjacent to the target, each taking half its share.
      final half = split.shares[loc.index] / 2;
      split.shares[loc.index] = half;
      final at = before ? loc.index : loc.index + 1;
      split.children.insert(at, draggedPane);
      split.shares.insert(at, half);
    } else {
      // Cross axis: wrap the target tile in a new split of the other axis.
      final tile = loc.tile;
      split.children[loc.index] = DockSplit(
        axis,
        before ? [draggedPane, tile] : [tile, draggedPane],
        [0.5, 0.5],
      );
    }
  }
  simplify(root);
}

/// The DockSplit directly enclosing `panel`'s tile, that tile (a DockPane when
/// the panel sits alone, else the DockTabs holding it), and its index in the
/// split. Null when the panel is absent.
({DockSplit split, DockNode tile, int index})? _tileOf(
  DockSplit split,
  Panel panel,
) {
  for (var i = 0; i < split.children.length; i++) {
    final child = split.children[i];
    if (!panelsIn(child).contains(panel)) continue;
    if (child is DockSplit) return _tileOf(child, panel);
    return (split: split, tile: child, index: i);
  }
  return null;
}

/// Remove `panel`'s pane wherever it sits, redistributing a split child's
/// share proportionally over its siblings and clamping a tab group's active
/// index. Returns whether it was found.
bool _removePanel(DockNode node, Panel panel) {
  switch (node) {
    case DockPane():
      return false;
    case DockTabs(:final children):
      final i = children.indexWhere((c) => c.panel == panel);
      if (i < 0) return false;
      children.removeAt(i);
      if (children.isNotEmpty) {
        node.active = node.active.clamp(0, children.length - 1);
      }
      return true;
    case DockSplit(:final children):
      for (var i = 0; i < children.length; i++) {
        final child = children[i];
        if (child is DockPane && child.panel == panel) {
          _removeSplitChild(node, i);
          return true;
        }
        if (_removePanel(child, panel)) return true;
      }
      return false;
  }
}

/// Drop child `i` from `split`, spreading its share over the survivors so the
/// total is preserved.
void _removeSplitChild(DockSplit split, int i) {
  final freed = split.shares[i];
  split.children.removeAt(i);
  split.shares.removeAt(i);
  if (split.shares.isEmpty) return;
  final total = split.shares.reduce((a, b) => a + b);
  if (total <= 0) {
    final equal = 1.0 / split.shares.length;
    for (var k = 0; k < split.shares.length; k++) {
      split.shares[k] = equal;
    }
  } else {
    for (var k = 0; k < split.shares.length; k++) {
      split.shares[k] += freed * split.shares[k] / total;
    }
  }
}

/// The dock's simplification rules, mirroring egui_tiles' options in
/// dock.rs::dock_simplification_options: prune empty tabs and splits, unwrap a
/// single-child tab group to a bare pane (K-086) and a single-child split to
/// its child, and join a nested split into a same-axis parent (scaling the
/// nested child's shares by its own share). The root always stays a DockSplit;
/// were it to collapse to one child, it keeps a one-child split instead.
void simplify(DockSplit root) {
  final result = _simplifyNode(root);
  if (identical(result, root)) return;
  root.children.clear();
  root.shares.clear();
  if (result != null) {
    root.children.add(result);
    root.shares.add(1.0);
  }
}

/// Simplify a node, returning its replacement: the same node (mutated), a
/// different node (when it unwraps), or null (when it prunes away).
DockNode? _simplifyNode(DockNode node) {
  switch (node) {
    case DockPane():
      return node;
    case DockTabs():
      if (node.children.isEmpty) return null;
      if (node.children.length == 1) return node.children.first;
      node.active = node.active.clamp(0, node.children.length - 1);
      return node;
    case DockSplit():
      final children = <DockNode>[];
      final shares = <double>[];
      for (var i = 0; i < node.children.length; i++) {
        final simplified = _simplifyNode(node.children[i]);
        if (simplified == null) continue;
        if (simplified is DockSplit && simplified.axis == node.axis) {
          // Join a same-axis nested split, scaling its shares by its own.
          final total = simplified.shares.reduce((a, b) => a + b);
          for (var k = 0; k < simplified.children.length; k++) {
            children.add(simplified.children[k]);
            shares.add(node.shares[i] *
                (total <= 0
                    ? 1.0 / simplified.shares.length
                    : simplified.shares[k] / total));
          }
        } else {
          children.add(simplified);
          shares.add(node.shares[i]);
        }
      }
      if (children.isEmpty) return null;
      if (children.length == 1) return children.first;
      node.children
        ..clear()
        ..addAll(children);
      node.shares
        ..clear()
        ..addAll(shares);
      return node;
  }
}
