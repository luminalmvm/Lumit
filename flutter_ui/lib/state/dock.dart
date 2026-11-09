// The dock layout model, ported from crates/lumit-ui/src/shell/dock.rs
// (which leans on egui_tiles). A serialisable tree of splits, tab groups and
// panes; the widget layer renders it and the model owns the invariants.
//
// In plain terms: the workspace is a tree. A *split* lays its children side by
// side (or stacked) with weighted shares; a *tabs* node stacks panels behind
// one another with a tab bar; a *pane* is one panel. A pane that sits alone —
// not inside a tabs node — renders bare, with no tab bar.

import 'dart:ui' show Rect;

import 'package:lumit_flutter/l10n/strings.dart';

/// The dockable panels — glossary names (docs/01-GLOSSARY.md §7).

enum Panel {
  project,
  viewer,
  timeline,
  effectControls,
  effectsAndPresets,
  scopes,

  /// The selected layer's grade as wheels, primaries, curves and a look,
  /// over the Colour effects that do the work. The Colour workspace's own.
  grade,
  debug,
  hierarchy,
  easing,

  /// The layer's effect stack drawn as nodes and wires, plus the drivers wired
  /// into its parameters. A second *view* of the same document, not a
  /// second document.
  graph,

  /// The parameter rows of whichever box the Graph panel has picked —
  /// the Nodes workspace's lower-right column. The Effect controls panel lists
  /// the whole stack; this one answers "what is selected", drivers included.
  node,

  /// The desk (docs/09 §3.1): one strip per audible row of the
  /// fronted comp — fader, pan pot, meters off the engine's tap, mute and
  /// solo — and the Master strip with its limiter lamp. The Audio workspace's
  /// left column fronts it.
  mixer,

  /// The Audio panel (docs/09, the approved AudioWorkspace board): output
  /// levels with the clip lamp, the Beats section over the beat engine, and
  /// the selected layer's sound — volume, pan, fades, and the two
  /// graph-template buttons.
  audio,

  /// The sound table (docs/impl/audio-timeline.md §5): one row per layer that
  /// can make a sound, each two lane rows tall, with the wave or the
  /// spectrogram on the lane and the clips laid along it. It stands where the
  /// Timeline stands in the Audio arrangement and keeps its own zoom, scroll
  /// and twirls, so the two tables can be open together.
  audioTimeline,

  /// The Cut timeline: the fronted comp arranged for cutting long footage.
  /// Every row is a Sequence layer, picture rows above a heavier line and
  /// audio-only rows below it, with the clips painted along each. A view over
  /// the same document the Timeline shows, as the Audio timeline is.
  cutTimeline,

  /// The font, size, spacing and outline of a text layer's letters, or of the
  /// next text the Type tool makes when no text layer is selected.
  text,

  /// Which side a text layer's lines line up on, and the room round them.
  paragraph,

  /// The user's saved expressions, and the editor that puts one on the
  /// selected properties.
  expressions,

  /// Starts the selected layers a set number of frames apart, and sets the
  /// order a stagger drag follows.
  stagger,

  /// One text area for the project, kept in the `.lum`.
  notes,

  /// Moves the selected layers' anchors to a point of their boxes, or lines
  /// that point up with the same point of the composition.
  align,

  /// An installed extension's own page. One pane for each extension, told
  /// apart by the pane's instance number, which `ExtensionService` gives
  /// each extension once and keeps.
  extension,

  /// How fast the selected layer plays and the commands that change it: the
  /// Retiming workspace's right-hand column heads with it.
  retime;

  String get title => switch (this) {
        Panel.project => l10n.panelProject,
        Panel.viewer => l10n.panelViewer,
        Panel.timeline => l10n.panelTimeline,
        Panel.effectControls => l10n.effectControls,
        Panel.effectsAndPresets => l10n.panelEffectsAndPresets,
        Panel.scopes => l10n.panelScopes,
        Panel.grade => l10n.panelGrade,
        Panel.hierarchy => l10n.panelHierarchy,
        Panel.easing => l10n.panelEasing,
        Panel.graph => l10n.panelGraph,
        Panel.node => l10n.panelNode,
        Panel.mixer => l10n.panelMixer,
        Panel.audio => l10n.panelAudio,
        Panel.audioTimeline => l10n.panelAudioTimeline,
        Panel.cutTimeline => l10n.panelCutTimeline,
        Panel.text => l10n.panelText,
        Panel.paragraph => l10n.panelParagraph,
        Panel.expressions => l10n.panelExpressions,
        Panel.stagger => l10n.panelStagger,
        Panel.retime => l10n.retime,
        Panel.notes => l10n.panelNotes,
        Panel.align => l10n.panelAlign,
        Panel.debug => l10n.panelDebug,
        Panel.extension => l10n.panelExtension,
      };
}

/// What the extension shown in the pane numbered `slot` is called, or null
/// when none is installed there. Set by `ExtensionService`, which is the one
/// thing that knows: an arrangement holds only the number.
String? Function(int slot)? extensionPaneTitle;

/// What a pane's tab says. The panel's own name, and for an extension's
/// pane the extension's.
String paneTitle(PaneId pane) => pane.panel == Panel.extension
    ? extensionPaneTitle?.call(pane.instance) ?? pane.panel.title
    : pane.panel.title;

/// Which pane, when a panel can be in the arrangement more than once.
///
/// The dock used to identify a pane by its [Panel] alone, and kept the
/// invariant that every panel appears exactly once. The Viewer broke that: a
/// comp in one picture and its precomp in another is two Viewer panes
/// (docs/impl/multi-viewer.md §3.1). So a pane is its panel plus an instance
/// number, and only the Viewer is ever given one above zero.
///
/// Instance 0 is what every panel that appears once has, and it is left out of
/// the saved arrangement, so a workspace with one Viewer writes exactly the
/// bytes it always did and one written by an older build reads back unchanged.
typedef PaneId = ({Panel panel, int instance});

/// This panel's first (usually only) pane.
extension PanelPane on Panel {
  PaneId pane([int instance = 0]) => (panel: this, instance: instance);
}

enum DockAxis { horizontal, vertical }

sealed class DockNode {
  Map<String, dynamic> toJson();

  /// Read a saved arrangement back. `null` for a node this build cannot make
  /// — a pane naming a panel that no longer exists, or a group left empty by
  /// one — and its parent drops it.
  ///
  /// **A folded-away panel must not cost anyone their arrangement.** Panels do
  /// go: the Node preview became a chip on the Viewer's own picture.
  /// Every workspace saved while it existed still names it, and reading that
  /// as a fault would have thrown on the way in — a stored layout taking the
  /// settings down with it, which is the worst way to learn a panel was
  /// removed. Dropped, the arrangement opens as it was minus the panel that
  /// has gone, which is exactly what it now means.
  static DockNode? fromJson(Map<String, dynamic> j) => switch (j['kind']) {
        'pane' => switch (Panel.values.asNameMap()[j['panel']]) {
            // A missing or nonsense instance is the first one, which is what
            // every arrangement written before panes had numbers means.
            final panel? => DockPane(panel, instance: _instance(j['n'])),
            _ => null,
          },
        'tabs' => _tabs(j),
        'split' => _split(j),
        _ => throw FormatException('unknown dock node: ${j['kind']}'),
      };

  static DockNode? _tabs(Map<String, dynamic> j) {
    final raw = j['children'] as List;
    // A stack's open panes and their heights are written against the children
    // as saved, so they are read before a dropped child shifts the rest.
    final wasOpen = {
      for (final i in j['open'] as List? ?? const [])
        if (i is int) i,
    };
    final heights = j['heights'] as List? ?? const [];
    final children = <DockPane>[];
    final open = <PaneId>{};
    final shares = <PaneId, double>{};
    for (var i = 0; i < raw.length; i++) {
      final pane = fromJson(raw[i] as Map<String, dynamic>);
      if (pane is! DockPane) continue;
      children.add(pane);
      if (wasOpen.contains(i)) open.add(pane.id);
      final height = i < heights.length ? heights[i] : null;
      if (height is num && height.isFinite && height > 0) {
        shares[pane.id] = height.toDouble();
      }
    }
    if (children.isEmpty) return null;
    // Clamped, because the tab that was fronted may be one of the dropped
    // ones — and a group opening on a tab that is not there is a blank panel.
    final active = (j['active'] as int? ?? 0).clamp(0, children.length - 1);
    return DockTabs(
      children,
      active: active,
      stacked: j['stacked'] == true,
      solo: j['solo'] == true,
      open: open,
      shares: shares,
    );
  }

  static DockNode? _split(Map<String, dynamic> j) {
    // The shares are positional, so a dropped child takes its own share with
    // it rather than leaving the list a different length from the children
    // (which the constructor asserts on, and rightly).
    final raw = j['children'] as List;
    final weights = j['shares'] as List;
    final children = <DockNode>[];
    final shares = <double>[];
    for (var i = 0; i < raw.length; i++) {
      final child = fromJson(raw[i] as Map<String, dynamic>);
      if (child == null) continue;
      children.add(child);
      shares.add(i < weights.length ? (weights[i] as num).toDouble() : 1);
    }
    if (children.isEmpty) return null;
    final split = DockSplit(
      j['axis'] == 'vertical' ? DockAxis.vertical : DockAxis.horizontal,
      children,
      shares,
    );
    if (j['windows'] case final List list) {
      final seen = panesIn(split).toSet();
      for (final raw in list) {
        final window = DockWindow.fromJson(raw);
        if (window == null || split.windows.any((w) => w.id == window.id)) {
          continue;
        }
        // A pane is in one place only. A file that says otherwise keeps the
        // first place it names.
        for (final pane in panesIn(window.root)) {
          if (!seen.add(pane)) _removePanel(window.root, pane);
        }
        _simplifyRoot(window.root);
        if (window.root.children.isNotEmpty) split.windows.add(window);
      }
    }
    return split;
  }
}

/// An instance number out of a saved arrangement: a whole number at least
/// zero, and zero for anything else.
int _instance(Object? raw) =>
    raw is num && raw.isFinite && raw >= 0 ? raw.toInt() : 0;

class DockPane extends DockNode {
  final Panel panel;

  /// Which pane of this panel, for the panels that can appear more than once
  /// (the Viewer, and only the Viewer). Zero for every other.
  final int instance;

  DockPane(this.panel, {this.instance = 0});

  PaneId get id => (panel: panel, instance: instance);

  @override
  Map<String, dynamic> toJson() => {
        'kind': 'pane',
        'panel': panel.name,
        // Left out at zero, so an arrangement with one Viewer serialises to
        // the bytes it always did.
        if (instance != 0) 'n': instance,
      };
}

/// A tab group. Children are panes (egui_tiles allows nesting, but the
/// shipped frontend only ever tabs panes — the port models what ships).
///
/// **A group can be drawn as a stack instead** ([stacked]): its panels one
/// above another, each under a header that twirls it open or shut, the way
/// After Effects stacks its side panels. It is the same group either way, so
/// a panel is dragged into a stack exactly as it is dragged into a tab group.
class DockTabs extends DockNode {
  final List<DockPane> children;
  int active;

  /// Drawn as a stack of twirled panels rather than as tabs.
  bool stacked;

  /// In a stack, opening one panel shuts the others.
  bool solo;

  /// The panes twirled open in a stack. By pane rather than by position, so
  /// reordering the stack moves nothing open or shut.
  final Set<PaneId> open;

  /// How much of the stack's height each open pane takes, as a weight against
  /// the other open ones. A pane with no entry weighs 1.
  final Map<PaneId, double> shares;

  DockTabs(
    this.children, {
    this.active = 0,
    this.stacked = false,
    this.solo = false,
    Set<PaneId>? open,
    Map<PaneId, double>? shares,
  })  : open = open ?? {},
        shares = shares ?? {};

  DockPane get activePane => children[active.clamp(0, children.length - 1)];

  bool isOpen(PaneId pane) => open.contains(pane);

  double shareOf(PaneId pane) => shares[pane] ?? 1;

  /// Front child [i]. In a stack that also twirls it open, since a panel
  /// somebody asked for is one they want to see.
  void front(int i) {
    active = i.clamp(0, children.length - 1);
    if (stacked) setOpen(children[active].id, true);
  }

  /// Twirl [pane] open or shut. With [solo] on, opening one shuts the rest.
  void setOpen(PaneId pane, bool value) {
    if (!value) {
      open.remove(pane);
      return;
    }
    if (solo) open.clear();
    open.add(pane);
  }

  @override
  Map<String, dynamic> toJson() => {
        'kind': 'tabs',
        'active': active,
        'children': [for (final c in children) c.toJson()],
        // Left out of a plain tab group, so one writes the bytes it always
        // did and a build from before stacks reads a stack as tabs.
        if (stacked) ...{
          'stacked': true,
          if (solo) 'solo': true,
          'open': [
            for (var i = 0; i < children.length; i++)
              if (open.contains(children[i].id)) i,
          ],
          'heights': [for (final c in children) shareOf(c.id)],
        },
      };
}

class DockSplit extends DockNode {
  final DockAxis axis;
  final List<DockNode> children;

  /// Weighted shares, same length as children, normalised on use.
  final List<double> shares;

  /// The floating windows, each holding panels taken out of the main one.
  /// Only the arrangement's root has any: a window is part of the
  /// arrangement, so it saves, restores and changes with the workspace and
  /// the project as the main window's panels do.
  final List<DockWindow> windows;

  DockSplit(this.axis, this.children, this.shares, {List<DockWindow>? windows})
      : windows = windows ?? [],
        assert(children.length == shares.length);

  @override
  Map<String, dynamic> toJson() => {
        'kind': 'split',
        'axis': axis.name,
        // A copy: the live list is mutated in place as splitters are dragged,
        // and a caller that keeps this map — the per-project session
        // does — would otherwise be holding the layout rather than a record of
        // what it was.
        'shares': [...shares],
        'children': [for (final c in children) c.toJson()],
        // Left out when there are none, so an arrangement in one window
        // writes the bytes it always did.
        if (windows.isNotEmpty) 'windows': [for (final w in windows) w.toJson()],
      };
}

/// Where a floating window sits on the screen, in the units the system
/// places windows in: physical pixels on Windows and Linux, points on macOS.
///
/// [bounds] is the window as it is when neither maximised nor full screen,
/// so putting either back lands where it was. [display] is the display it was
/// on, which is how a frame from a monitor that has since gone is spotted.
class FloatingFrame {
  final Rect bounds;
  final bool maximised;
  final bool fullScreen;
  final Rect? display;

  const FloatingFrame(
    this.bounds, {
    this.maximised = false,
    this.fullScreen = false,
    this.display,
  });

  Map<String, dynamic> toJson() => {
        'x': bounds.left,
        'y': bounds.top,
        'w': bounds.width,
        'h': bounds.height,
        if (maximised) 'maximised': true,
        if (fullScreen) 'full_screen': true,
        if (display case final d?) 'display': [d.left, d.top, d.width, d.height],
      };

  /// Null for anything that is not a finite rectangle of some size, which a
  /// hand-edited file can hold: the window then opens where a new one would.
  static FloatingFrame? fromJson(Object? raw) {
    if (raw is! Map) return null;
    final x = raw['x'], y = raw['y'], w = raw['w'], h = raw['h'];
    if (x is! num || y is! num || w is! num || h is! num) return null;
    final bounds = Rect.fromLTWH(
        x.toDouble(), y.toDouble(), w.toDouble(), h.toDouble());
    if (!bounds.isFinite || bounds.width < 1 || bounds.height < 1) return null;
    return FloatingFrame(
      bounds,
      maximised: raw['maximised'] == true,
      fullScreen: raw['full_screen'] == true,
      display: _rect(raw['display']),
    );
  }

  static Rect? _rect(Object? raw) {
    if (raw is! List || raw.length != 4 || raw.any((n) => n is! num)) {
      return null;
    }
    final r = Rect.fromLTWH((raw[0] as num).toDouble(),
        (raw[1] as num).toDouble(), (raw[2] as num).toDouble(),
        (raw[3] as num).toDouble());
    return r.isFinite && r.width > 0 && r.height > 0 ? r : null;
  }

  @override
  bool operator ==(Object other) =>
      other is FloatingFrame &&
      other.bounds == bounds &&
      other.maximised == maximised &&
      other.fullScreen == fullScreen &&
      other.display == display;

  @override
  int get hashCode => Object.hash(bounds, maximised, fullScreen, display);
}

/// Where a pane in a floating window came from, so it can be put back: beside
/// [beside], at [at], in the split [depth] levels below the root, taking
/// [share] of it. In a group, [index] is the tab it was.
class DockHome {
  final PaneId beside;
  final DropPosition at;
  final int depth;
  final double share;
  final int index;

  const DockHome(this.beside, this.at,
      {this.depth = 0, this.share = 0.5, this.index = -1});

  Map<String, dynamic> toJson() => {
        'beside': _paneJson(beside),
        'at': at.name,
        'depth': depth,
        'share': share,
        if (index >= 0) 'index': index,
      };

  static DockHome? fromJson(Object? raw) {
    if (raw is! Map) return null;
    final beside = _paneFromJson(raw['beside']);
    final at = DropPosition.values.asNameMap()[raw['at']];
    if (beside == null || at == null) return null;
    final depth = raw['depth'], share = raw['share'], index = raw['index'];
    return DockHome(
      beside,
      at,
      depth: depth is num && depth >= 0 ? depth.toInt() : 0,
      share: share is num && share.isFinite && share > 0 && share <= 1
          ? share.toDouble()
          : 0.5,
      index: index is num && index >= 0 ? index.toInt() : -1,
    );
  }
}

Map<String, dynamic> _paneJson(PaneId pane) => {
      'panel': pane.panel.name,
      if (pane.instance != 0) 'n': pane.instance,
    };

PaneId? _paneFromJson(Object? raw) {
  if (raw is! Map) return null;
  final panel = Panel.values.asNameMap()[raw['panel']];
  return panel?.pane(_instance(raw['n']));
}

/// A floating window: an arrangement of its own beside the main window's.
class DockWindow {
  /// What the window is known by while it lasts and across restarts: its
  /// view is keyed by it, and a window's place is remembered against it.
  final int id;

  /// Its panels, arranged as the main window's are.
  final DockSplit root;

  /// Where it was last seen, or null for a window never yet placed.
  FloatingFrame? frame;

  /// Kept above every other program's windows, not only Lumit's.
  bool onTop;

  /// Where each pane in it came from (see [dockPane]). Kept in step with the
  /// panes it holds by [simplify].
  final Map<PaneId, DockHome> homes;

  DockWindow(this.id, this.root,
      {this.frame, this.onTop = false, Map<PaneId, DockHome>? homes})
      : homes = homes ?? {};

  Map<String, dynamic> toJson() => {
        'id': id,
        'root': root.toJson(),
        if (frame case final f?) 'frame': f.toJson(),
        if (onTop) 'on_top': true,
        if (homes.isNotEmpty)
          'homes': [
            for (final e in homes.entries)
              {'pane': _paneJson(e.key), ...e.value.toJson()},
          ],
      };

  /// Null when there is nothing in it this build can show, which drops the
  /// window rather than the arrangement.
  static DockWindow? fromJson(Object? raw) {
    if (raw is! Map) return null;
    final id = raw['id'];
    final tree = raw['root'];
    if (id is! num || tree is! Map) return null;
    final DockNode? parsed;
    try {
      parsed = DockNode.fromJson(tree.cast<String, dynamic>());
    } catch (_) {
      return null;
    }
    final root = switch (parsed) {
      DockSplit split => split,
      null => null,
      final node => DockSplit(DockAxis.horizontal, [node], [1.0]),
    };
    if (root == null || root.children.isEmpty) return null;
    final homes = <PaneId, DockHome>{};
    if (raw['homes'] case final List list) {
      for (final h in list) {
        final pane = h is Map ? _paneFromJson(h['pane']) : null;
        final home = DockHome.fromJson(h);
        if (pane != null && home != null) homes[pane] = home;
      }
    }
    return DockWindow(
      id.toInt(),
      root,
      frame: FloatingFrame.fromJson(raw['frame']),
      onTop: raw['on_top'] == true,
      homes: homes,
    );
  }
}

/// The default workspace (docs/07 §1.6 "Edit"): a vertical root (upper band
/// 0.68, Timeline 0.32 across the full width); the upper band horizontal
/// (left tab group 0.22, Viewer 0.58, right group 0.20). The left group
/// tabs Project (fronted), Effect controls and Notes; the right group is a
/// **stack**: Effects & presets twirled open, with Text, Paragraph, Align and
/// Stagger shut beneath it, each one click away. Viewer and Timeline sit
/// alone and render bare. Scopes is Colour's and Effects', not Edit's.
///
/// **The Debug panel is in no shipped arrangement either**: it is a
/// developer's readout, and the owner took it out of every default when the
/// arrangements were measured off their own screen. Like Hierarchy it is one
/// tick away in the Window menu.
///
/// **The Hierarchy panel is in no shipped arrangement**: it is the
/// parenting tree, which is a thing you go and look at rather than a thing
/// you work beside, and a tab nobody opens is a tab in everybody's way. It
/// is one tick away in the Window menu, like Easing, Graph and Node.
DockSplit defaultLayout() => DockSplit(
      DockAxis.vertical,
      [
        DockSplit(
          DockAxis.horizontal,
          [
            DockTabs([
              DockPane(Panel.project),
              DockPane(Panel.effectControls),
              DockPane(Panel.notes),
            ]),
            DockPane(Panel.viewer),
            DockTabs(
              [
                DockPane(Panel.effectsAndPresets),
                DockPane(Panel.text),
                DockPane(Panel.paragraph),
                DockPane(Panel.align),
                DockPane(Panel.stagger),
              ],
              stacked: true,
              open: {Panel.effectsAndPresets.pane()},
            ),
          ],
          [0.22, 0.58, 0.20],
        ),
        DockPane(Panel.timeline),
      ],
      [0.68, 0.32],
    );

/// The shipped workspace presets (docs/07 §1.6): much the same panel
/// inventory, arranged for different work. Structure only, per the spec.
///
/// Each one carries the panels its work reaches for, and the ones it reaches
/// for less sit shut in a stack rather than being left out: a shut panel
/// costs one header line. Project and Effect controls stay full height, tabbed
/// on the left, wherever a workspace has a left column. Easing is Effects' and
/// Retiming's, Graph and Node are Nodes', and the Audio panel is Audio's.
/// Hierarchy is in none of them, as it is a thing you go and look at rather
/// than work beside.
///
/// The order is the strip's order, which is the drawing's: Nodes sits third,
/// beside Effects, because both are about what an effect does rather than
/// where a layer sits.
enum WorkspacePreset {
  edit,
  effects,
  nodes,
  colour,
  audio,
  retiming,

  /// Last, so the strip's slots and chords of the six before it stay where
  /// they were.
  cut;

  String get title => switch (this) {
        WorkspacePreset.edit => l10n.workspaceEdit,
        WorkspacePreset.effects => l10n.workspaceEffects,
        WorkspacePreset.nodes => l10n.workspaceNodes,
        WorkspacePreset.colour => l10n.workspaceColour,
        WorkspacePreset.audio => l10n.workspaceAudio,
        WorkspacePreset.retiming => l10n.workspaceRetiming,
        WorkspacePreset.cut => l10n.workspaceCut,
      };
}

/// The factory layout of one preset — restorable at any time, never touched
/// by the user's own rearranging (that persists separately).
DockSplit presetLayout(WorkspacePreset preset) => switch (preset) {
      // Edit is the default arrangement.
      WorkspacePreset.edit => defaultLayout(),
      // Project and Effect controls (fronted) tabbed on the left; the Easing
      // panel full height beside the Viewer; then a stack at the right edge
      // with Effects & presets open, and Expressions and Scopes shut. The
      // Timeline is Edit's height exactly: the owner asked for one
      // timeline height across Edit, Effects, Nodes and Colour, so that
      // changing workspace does not move the lanes under the pointer. The
      // stack is the owner's own measured width — wide enough to work
      // in rather than as narrow as an exactly 16:9 Viewer would need.
      WorkspacePreset.effects => DockSplit(
          DockAxis.vertical,
          [
            DockSplit(
              DockAxis.horizontal,
              [
                DockTabs([
                  DockPane(Panel.project),
                  DockPane(Panel.effectControls),
                ], active: 1),
                DockPane(Panel.viewer),
                DockPane(Panel.easing),
                DockTabs(
                  [
                    DockPane(Panel.effectsAndPresets),
                    DockPane(Panel.expressions),
                    DockPane(Panel.scopes),
                  ],
                  stacked: true,
                  open: {Panel.effectsAndPresets.pane()},
                ),
              ],
              [0.20, 0.5088, 0.16, 0.1312],
            ),
            DockPane(Panel.timeline),
          ],
          [0.68, 0.32],
        ),
      // Nodes: the graph as the main surface, and the one
      // preset whose root splits **across** rather than down — the Timeline
      // runs under the Graph panel only, not under the small viewer, which is
      // what the approved Nodes-workspace drawing shows. The proportions are
      // the owner's own, measured off their screen and superseding the
      // drawing's: 0.76/0.24 across; the graph column 0.68/0.32, so
      // the Timeline stands at the height every other arrangement gives it
      // rather than the drawing's short strip; the right column 0.32 Viewer —
      // whole bar kept — to 0.68 for a stack with the Node panel open, the
      // parameter rows being what this workspace is for, and Effects &
      // presets and Expressions shut under it, one click from the graph.
      WorkspacePreset.nodes => DockSplit(
          DockAxis.horizontal,
          [
            DockSplit(
              DockAxis.vertical,
              [DockPane(Panel.graph), DockPane(Panel.timeline)],
              [0.68, 0.32],
            ),
            DockSplit(
              DockAxis.vertical,
              [
                DockPane(Panel.viewer),
                DockTabs(
                  [
                    DockPane(Panel.node),
                    DockPane(Panel.effectsAndPresets),
                    DockPane(Panel.expressions),
                  ],
                  stacked: true,
                  open: {Panel.node.pane()},
                ),
              ],
              [0.3169, 0.6831],
            ),
          ],
          [0.76, 0.24],
        ),
      // Project and Effects & presets tabbed on the left; the Viewer in the
      // centre with Scopes under it, wide, the way a waveform wants to be
      // read; the Grade panel full height on the right with Effect controls
      // one tab behind it; the Timeline at the one height.
      WorkspacePreset.colour => DockSplit(
          DockAxis.vertical,
          [
            DockSplit(
              DockAxis.horizontal,
              [
                DockTabs([
                  DockPane(Panel.project),
                  DockPane(Panel.effectsAndPresets),
                ]),
                DockSplit(
                  DockAxis.vertical,
                  [
                    DockPane(Panel.viewer),
                    DockPane(Panel.scopes),
                  ],
                  [0.62, 0.38],
                ),
                DockTabs([
                  DockPane(Panel.grade),
                  DockPane(Panel.effectControls),
                ]),
              ],
              [0.16, 0.56, 0.28],
            ),
            DockPane(Panel.timeline),
          ],
          [0.68, 0.32],
        ),
      // Audio (the approved AudioWorkspace board): the Mixer fronting the
      // left column over Project and Effect controls, the Audio panel open in
      // a stack on the right with Effects & presets shut under it, the
      // Viewer reduced between them, and the **Audio timeline** across the
      // bottom where every other arrangement puts the Timeline - the sound
      // table, with a row per audible layer. Shares are the board's own
      // proportions: 260 / 840 / 340 across a 1440 drawing, the bottom band
      // 420 of 900.
      WorkspacePreset.audio => DockSplit(
          DockAxis.vertical,
          [
            DockSplit(
              DockAxis.horizontal,
              [
                DockTabs([
                  DockPane(Panel.mixer),
                  DockPane(Panel.project),
                  DockPane(Panel.effectControls),
                ]),
                DockPane(Panel.viewer),
                DockTabs(
                  [
                    DockPane(Panel.audio),
                    DockPane(Panel.effectsAndPresets),
                  ],
                  stacked: true,
                  open: {Panel.audio.pane()},
                ),
              ],
              [0.18, 0.58, 0.24],
            ),
            DockPane(Panel.audioTimeline),
          ],
          [0.53, 0.47],
        ),
      // Retiming: the arrangement for shaping movement. The Retime panel has
      // the right-hand column, with Easing, Stagger and Expressions shut under
      // it. Easing has nothing to do on the speed graph, so it opens when it is
      // wanted. The Timeline is as tall as Audio's and comes up on the speed
      // graph, because that is where the eye is (the Timeline does that part,
      // see `presetApplied`).
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
                ]),
                DockPane(Panel.viewer),
                DockTabs(
                  [
                    DockPane(Panel.retime),
                    DockPane(Panel.easing),
                    DockPane(Panel.stagger),
                    DockPane(Panel.expressions),
                  ],
                  stacked: true,
                  open: {Panel.retime.pane()},
                  shares: {Panel.retime.pane(): 1.4},
                ),
              ],
              [0.20, 0.58, 0.22],
            ),
            DockPane(Panel.timeline),
          ],
          [0.55, 0.45],
        ),
      // Cut: Project, Effect controls, Effects & presets and Notes tabbed
      // down the left, each full height. The Viewer
      // takes the rest of the top band (the Source view is a second view
      // inside it, so one pane serves), and the **Cut timeline** runs across
      // the bottom at the Audio arrangement's proportions, with the Mixer at
      // its right for levels and faders while cutting. The Project column is
      // a little wider than elsewhere, so its Duration column has room on a
      // 1920 screen.
      WorkspacePreset.cut => DockSplit(
          DockAxis.vertical,
          [
            DockSplit(
              DockAxis.horizontal,
              [
                DockTabs([
                  DockPane(Panel.project),
                  DockPane(Panel.effectControls),
                  DockPane(Panel.effectsAndPresets),
                  DockPane(Panel.notes),
                ]),
                DockPane(Panel.viewer),
              ],
              [0.24, 0.76],
            ),
            DockSplit(
              DockAxis.horizontal,
              [DockPane(Panel.cutTimeline), DockPane(Panel.mixer)],
              [0.82, 0.18],
            ),
          ],
          [0.53, 0.47],
        ),
    };

/// Every pane present in the tree, in visit order, the floating windows' after
/// the main window's. This is the one the invariants are stated over: a pane
/// appears exactly once, and a panel may appear more than once only through
/// its instance number.
List<PaneId> panesIn(DockNode node) => switch (node) {
      DockPane pane => [pane.id],
      DockTabs(:final children) => [for (final c in children) c.id],
      DockSplit(:final children, :final windows) => [
          for (final c in children) ...panesIn(c),
          for (final w in windows) ...panesIn(w.root),
        ],
    };

/// The panes in the main window alone.
List<PaneId> mainPanesIn(DockSplit root) =>
    [for (final c in root.children) ...panesIn(c)];

/// The floating window holding [pane], or null when it is in the main window
/// or nowhere.
DockWindow? windowOf(DockSplit root, PaneId pane) {
  for (final w in root.windows) {
    if (panesIn(w.root).contains(pane)) return w;
  }
  return null;
}


/// Every panel present in the tree, in visit order, a panel with two panes
/// appearing twice. What the Window menu's ticks and the width sweep read.
List<Panel> panelsIn(DockNode node) =>
    [for (final pane in panesIn(node)) pane.panel];

/// The next free instance number for `panel` — what a new Viewer is given.
int nextInstance(DockNode node, Panel panel) {
  var next = 0;
  for (final pane in panesIn(node)) {
    if (pane.panel == panel && pane.instance >= next) next = pane.instance + 1;
  }
  return next;
}

/// Whether `panel` is anywhere in the tree — which is what "visible" means for
/// a dock: a panel that is not in the arrangement is not on screen, and one
/// that is can always be brought to the front of its group.
bool panelVisible(DockNode node, Panel panel) => panelsIn(node).contains(panel);

/// Add or drop `panel`, for the Window menu's tick list. A no-op when the tree
/// already agrees with `visible`.
///
/// Showing puts it into a stack when the arrangement has one, twirled open,
/// and otherwise into the first tab group, fronted — a panel you just asked
/// for is the one you want to look at. With no group at all it pairs up
/// with the first tile instead, so it never has to invent a share of the
/// window. Hiding drops the pane and simplifies, exactly as closing a tab does.
/// The main window's last panel cannot be hidden: an empty dock has no way
/// back. A panel shown is shown in the main window.
void setPanelVisible(DockSplit root, Panel panel, bool visible) {
  if (panelsIn(root).contains(panel) == visible) return;
  if (!visible) {
    if (mainPanesIn(root).every((p) => p.panel == panel)) return;
    // Every pane of it, so ticking the Viewer off closes all of them rather
    // than leaving the arrangement half agreeing with the menu.
    for (final pane in panesIn(root).where((p) => p.panel == panel).toList()) {
      _removePanel(root, pane);
    }
    simplify(root);
    return;
  }
  final tabs = _firstTabs(root, stacked: true) ?? _firstTabs(root);
  if (tabs != null) {
    tabs.children.add(DockPane(panel));
    tabs.front(tabs.children.length - 1);
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

/// Whether [panel] is the tab in front of a group that also holds [behind].
/// A stack shows every pane it has open, so nothing in one is behind.
bool frontedOver(DockNode node, Panel panel, Panel behind) => switch (node) {
      DockPane() => false,
      DockTabs(:final children) => !node.stacked &&
          node.activePane.panel == panel &&
          children.any((c) => c.panel == behind),
      DockSplit(:final children, :final windows) =>
        children.any((c) => frontedOver(c, panel, behind)) ||
            windows.any((w) => frontedOver(w.root, panel, behind)),
    };

/// The first tab group in visit order, or null when every panel sits alone.
/// With [stacked], the first group drawn as a stack.
DockTabs? _firstTabs(DockNode node, {bool stacked = false}) {
  switch (node) {
    case DockPane():
      return null;
    case DockTabs():
      return !stacked || node.stacked ? node : null;
    case DockSplit(:final children):
      for (final child in children) {
        final found = _firstTabs(child, stacked: stacked);
        if (found != null) return found;
      }
      return null;
  }
}

/// The group holding [pane], or null when it sits alone or is absent. What a
/// group's own menu and a stack's headers act on.
DockTabs? groupOf(DockNode node, PaneId pane) {
  switch (node) {
    case DockPane():
      return null;
    case DockTabs(:final children):
      return children.any((c) => c.id == pane) ? node : null;
    case DockSplit(:final children, :final windows):
      for (final child in [...children, for (final w in windows) w.root]) {
        final found = groupOf(child, pane);
        if (found != null) return found;
      }
      return null;
  }
}

/// Put `pane` into the arrangement if it is not there: into a stack when
/// there is one, and otherwise into the first tab group, fronted. What
/// showing an extension's panel does, where [setPanelVisible] would only
/// know the panel and not which extension.
void showPane(DockSplit root, PaneId pane) {
  if (panesIn(root).contains(pane)) return;
  final made = DockPane(pane.panel, instance: pane.instance);
  final tabs = _firstTabs(root, stacked: true) ?? _firstTabs(root);
  if (tabs != null) {
    tabs.children.add(made);
    tabs.front(tabs.children.length - 1);
    return;
  }
  final first = root.children.first;
  if (first is DockPane) {
    root.children[0] = DockTabs([first, made], active: 1);
    return;
  }
  root.children.insert(0, made);
  root.shares.insert(0, 0.2);
}

/// Close one pane, whichever tab group, split or window holds it, and
/// simplify. The main window's last pane cannot be closed: an empty dock has
/// no way back. A floating window's last pane takes the window with it.
///
/// What the Viewer's own close does, since a second Viewer goes without the
/// panel type going with it.
void closePane(DockSplit root, PaneId pane) {
  final main = mainPanesIn(root);
  if (main.length <= 1 && main.contains(pane)) return;
  if (!_removePanel(root, pane)) return;
  simplify(root);
}

/// Add another pane of `panel`, beside `beside` when it is given and in the
/// first tab group otherwise, and answer the pane it made.
///
/// Only the Viewer asks for this. Everything else is one pane and stays one.
PaneId addPane(DockSplit root, Panel panel, {PaneId? beside}) {
  final made = (panel: panel, instance: nextInstance(root, panel));
  final loc = beside == null ? null : _tileOf(root, beside);
  if (loc != null) {
    // Beside the pane it was asked for, splitting its share in two — the same
    // arithmetic a drop on that pane's right edge does.
    final half = loc.split.shares[loc.index] / 2;
    loc.split.shares[loc.index] = half;
    loc.split.children.insert(
      loc.index + 1,
      DockPane(panel, instance: made.instance),
    );
    loc.split.shares.insert(loc.index + 1, half);
    return made;
  }
  setPanelVisible(root, panel, true);
  // `setPanelVisible` only adds a pane when the panel is absent, so put the
  // new one in by hand when it is already there.
  if (!panesIn(root).contains(made)) {
    final tabs = _firstTabs(root);
    if (tabs != null) {
      tabs.children.add(DockPane(panel, instance: made.instance));
      tabs.front(tabs.children.length - 1);
    } else {
      root.children.insert(0, DockPane(panel, instance: made.instance));
      root.shares.insert(0, 0.2);
    }
  }
  return made;
}

/// Bring `panel`'s tab to the front of whichever tab group holds it (the
/// start-up "always open on Project" rule).
///
/// **Says whether anything moved**, so a caller can keep quiet when it did
/// not: fronting is asked for on every layer click (`LumitUiState.frontPanel`,
/// item 6.28), and the tab is nearly always fronted already — a front that
/// changes no tab must not repaint the shell or write the workspace to disk
/// (docs/impl/ui-performance.md §4.4).
bool activatePanelTab(DockNode node, Panel panel) {
  switch (node) {
    case DockPane():
      return false;
    case DockTabs(:final children):
      final i = children.indexWhere((c) => c.panel == panel);
      if (i < 0) return false;
      // In a stack, fronted means twirled open.
      final shut = node.stacked && !node.isOpen(children[i].id);
      if (node.active == i && !shut) return false;
      node.front(i);
      return true;
    case DockSplit(:final children, :final windows):
      var moved = false;
      for (final c in [...children, for (final w in windows) w.root]) {
        if (activatePanelTab(c, panel)) moved = true;
      }
      return moved;
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
///
/// The two may be in different windows. The main window's last pane stays
/// where it is, and a pane leaving the main window remembers where it was.
void movePanel(
  DockSplit root,
  PaneId dragged,
  PaneId target,
  DropPosition pos,
) {
  if (dragged == target) return;
  final present = panesIn(root).toSet();
  if (!present.contains(dragged) || !present.contains(target)) return;
  final from = windowOf(root, dragged);
  final to = windowOf(root, target);
  if (from == null && to != null && mainPanesIn(root).length <= 1) return;
  final home = to == null
      ? null
      : from == null
          ? _homeIn(root, dragged)
          : from.homes[dragged];

  _removePanel(root, dragged);
  from?.homes.remove(dragged);
  if (to != null && home != null) to.homes[dragged] = home;
  final loc = _tileOf(root, target);
  // The target should always survive the removal; bail defensively if not.
  if (loc == null) return;

  final draggedPane = DockPane(dragged.panel, instance: dragged.instance);
  if (pos == DropPosition.stack) {
    final tile = loc.tile;
    if (tile is DockTabs) {
      // A stack is read top to bottom, so a panel dropped on one lands under
      // the panel it was dropped on. A tab group takes it at the end.
      final at = tile.stacked
          ? tile.children.indexWhere((c) => c.id == target) + 1
          : tile.children.length;
      tile.children.insert(at, draggedPane);
      tile.front(at);
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
  PaneId pane,
) {
  for (var i = 0; i < split.children.length; i++) {
    final child = split.children[i];
    if (!panesIn(child).contains(pane)) continue;
    if (child is DockSplit) return _tileOf(child, pane);
    return (split: split, tile: child, index: i);
  }
  for (final w in split.windows) {
    final found = _tileOf(w.root, pane);
    if (found != null) return found;
  }
  return null;
}

/// Remove `panel`'s pane wherever it sits, redistributing a split child's
/// share proportionally over its siblings and clamping a tab group's active
/// index. Returns whether it was found.
bool _removePanel(DockNode node, PaneId pane) {
  switch (node) {
    case DockPane():
      return false;
    case DockTabs(:final children):
      final i = children.indexWhere((c) => c.id == pane);
      if (i < 0) return false;
      children.removeAt(i);
      if (children.isNotEmpty) {
        node.active = node.active.clamp(0, children.length - 1);
      }
      return true;
    case DockSplit(:final children, :final windows):
      for (var i = 0; i < children.length; i++) {
        final child = children[i];
        if (child is DockPane && child.id == pane) {
          _removeSplitChild(node, i);
          return true;
        }
        if (_removePanel(child, pane)) return true;
      }
      for (final w in windows) {
        if (_removePanel(w.root, pane)) return true;
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
/// single-child tab group to a bare pane and a single-child split to
/// its child, and join a nested split into a same-axis parent (scaling the
/// nested child's shares by its own share). The root always stays a DockSplit;
/// were it to collapse to one child, it keeps a one-child split instead.
///
/// Each floating window's tree is simplified the same way, and a window left
/// with nothing in it goes.
void simplify(DockSplit root) {
  _simplifyRoot(root);
  for (final w in root.windows) {
    _simplifyRoot(w.root);
  }
  root.windows.removeWhere((w) => w.root.children.isEmpty);
  for (final w in root.windows) {
    final held = panesIn(w.root).toSet();
    w.homes.removeWhere((pane, _) => !held.contains(pane));
  }
}

void _simplifyRoot(DockSplit root) {
  final result = _simplifyNode(root);
  if (identical(result, root)) return;
  root.children.clear();
  root.shares.clear();
  if (result != null) {
    root.children.add(result);
    root.shares.add(1.0);
  }
}

// --- Floating windows -------------------------------------------------------

/// A number no window in [root] has yet.
int nextWindowId(DockSplit root) => root.windows.fold(
    1, (next, w) => w.id >= next ? w.id + 1 : next);

/// Whether [pane] may be taken out into a window of its own: anywhere but
/// the main window's last pane, and not a pane already alone in a window.
bool canFloat(DockSplit root, PaneId pane) {
  final window = windowOf(root, pane);
  if (window != null) return panesIn(window.root).length > 1;
  final main = mainPanesIn(root);
  return main.contains(pane) && main.length > 1;
}

/// Take [pane] out into a new window, placed at [frame], and answer it. Null
/// when it cannot go ([canFloat]).
DockWindow? floatPane(DockSplit root, PaneId pane, {FloatingFrame? frame}) {
  if (!canFloat(root, pane)) return null;
  final from = windowOf(root, pane);
  final home = from == null ? _homeIn(root, pane) : from.homes[pane];
  _removePanel(root, pane);
  final window = DockWindow(
    nextWindowId(root),
    DockSplit(DockAxis.horizontal,
        [DockPane(pane.panel, instance: pane.instance)], [1.0]),
    frame: frame,
    homes: {if (home != null) pane: home},
  );
  root.windows.add(window);
  simplify(root);
  return window;
}

/// Take the whole group holding [pane] out into a new window, drawn the way
/// it was. Null when there is no group or nothing would be left behind.
DockWindow? floatGroup(DockSplit root, PaneId pane, {FloatingFrame? frame}) {
  final group = groupOf(root, pane);
  if (group == null) return null;
  final members = [for (final c in group.children) c.id];
  final from = windowOf(root, pane);
  final left = from == null
      ? mainPanesIn(root).where((p) => !members.contains(p)).length
      : panesIn(from.root).where((p) => !members.contains(p)).length;
  if (left == 0) return null;
  // The first member goes back where the group stood, the rest into it.
  final homes = <PaneId, DockHome>{};
  if (from == null) {
    final first = _homeOfTile(root, members.first);
    if (first != null) homes[members.first] = first;
    for (var i = 1; i < members.length; i++) {
      homes[members[i]] = DockHome(members.first, DropPosition.stack, index: i);
    }
  } else {
    for (final m in members) {
      if (from.homes[m] case final home?) homes[m] = home;
    }
  }
  final loc = _tileOf(root, pane);
  if (loc == null) return null;
  _removeSplitChild(loc.split, loc.index);
  final window = DockWindow(
    nextWindowId(root),
    DockSplit(DockAxis.horizontal, [group], [1.0]),
    frame: frame,
    homes: homes,
  );
  root.windows.add(window);
  simplify(root);
  return window;
}

/// Put [pane] back in the main window: where it came from when that is still
/// there to go beside, and otherwise where [showPane] puts a panel.
void dockPane(DockSplit root, PaneId pane) {
  final from = windowOf(root, pane);
  if (from == null) return;
  final home = from.homes.remove(pane);
  _removePanel(root, pane);
  if (home != null && mainPanesIn(root).contains(home.beside)) {
    _putBack(root, pane, home);
  } else {
    showPane(root, pane);
  }
  simplify(root);
}

/// Put every pane of window [id] back in the main window. A window that was
/// one group comes back as that group, drawn the way it was.
void dockWindow(DockSplit root, int id) {
  final at = root.windows.indexWhere((w) => w.id == id);
  if (at < 0) return;
  final window = root.windows[at];
  final only = window.root.children.length == 1 ? window.root.children.single : null;
  final panes = panesIn(window.root);
  for (final pane in panes) {
    // A pane whose home is beside another pane of this window goes after it,
    // which the visit order already gives.
    dockPane(root, pane);
  }
  if (only is DockTabs && panes.isNotEmpty) {
    final group = groupOf(root, panes.first);
    if (group != null &&
        group.children.length == only.children.length &&
        group.children.every((c) => panes.contains(c.id))) {
      group
        ..stacked = only.stacked
        ..solo = only.solo
        ..active = only.active.clamp(0, group.children.length - 1);
      group.open
        ..clear()
        ..addAll(only.open);
      group.shares
        ..clear()
        ..addAll(only.shares);
    }
  }
}

/// Every floating window's panes back in the main window.
void dockAllWindows(DockSplit root) {
  for (final w in root.windows.toList()) {
    dockWindow(root, w.id);
  }
}

/// [root]'s arrangement with every floating window's panes back in the main
/// window, as JSON, leaving [root] as it is. What travels inside a `.lum`:
/// where windows stand is this machine's business, and a project opened on
/// another opens in one window.
Map<String, dynamic> withoutWindows(DockSplit root) {
  if (root.windows.isEmpty) return root.toJson();
  final copy = DockNode.fromJson(root.toJson());
  if (copy is! DockSplit) return root.toJson();
  dockAllWindows(copy);
  return copy.toJson();
}

/// The splits from [root]'s main tree down to the one holding [pane]'s tile,
/// each with the index of the child the way goes through. Null when [pane]
/// is not in the main tree.
List<({DockSplit split, int index})>? _pathTo(DockSplit root, PaneId pane) {
  for (var i = 0; i < root.children.length; i++) {
    final child = root.children[i];
    if (!panesIn(child).contains(pane)) continue;
    if (child is DockSplit) {
      final rest = _pathTo(child, pane);
      return rest == null ? null : [(split: root, index: i), ...rest];
    }
    return [(split: root, index: i)];
  }
  return null;
}

/// Where [pane] sits in the main tree, said as the drop that would put it
/// back once it has gone: into its group beside the pane next to it, or
/// beside the tile next to it in the split it shares with that tile. Null
/// when it is alone in the tree.
DockHome? _homeIn(DockSplit root, PaneId pane) {
  final path = _pathTo(root, pane);
  if (path == null) return null;
  final last = path.last;
  final tile = last.split.children[last.index];
  if (tile is DockTabs) {
    final i = tile.children.indexWhere((c) => c.id == pane);
    final next = i + 1 < tile.children.length ? i + 1 : i - 1;
    if (next < 0) return null;
    return DockHome(tile.children[next].id, DropPosition.stack, index: i);
  }
  return _homeOfTile(root, pane);
}

/// Where the tile holding [pane] stands in its split, as a drop beside the
/// tile next to it.
DockHome? _homeOfTile(DockSplit root, PaneId pane) {
  final path = _pathTo(root, pane);
  if (path == null) return null;
  final (:split, :index) = path.last;
  if (split.children.length < 2) return null;
  final total = split.shares.fold<double>(0, (a, b) => a + b);
  final share = total <= 0 ? 0.5 : split.shares[index] / total;
  final after = index > 0;
  final neighbour = split.children[after ? index - 1 : index + 1];
  final horizontal = split.axis == DockAxis.horizontal;
  return DockHome(
    after ? panesIn(neighbour).last : panesIn(neighbour).first,
    after
        ? (horizontal ? DropPosition.right : DropPosition.below)
        : (horizontal ? DropPosition.left : DropPosition.above),
    depth: path.length - 1,
    share: share,
  );
}

/// Put [pane], which is in no tree, back per [home]: in the split it came
/// out of when the tree still has one that deep on the way to its
/// neighbour, and otherwise against the neighbour's own tile.
void _putBack(DockSplit root, PaneId pane, DockHome home) {
  final made = DockPane(pane.panel, instance: pane.instance);
  final loc = _tileOf(root, home.beside);
  if (loc == null) return;
  if (home.at == DropPosition.stack) {
    final tile = loc.tile;
    if (tile is DockTabs) {
      final at = home.index < 0
          ? tile.children.length
          : home.index.clamp(0, tile.children.length);
      tile.children.insert(at, made);
      tile.front(at);
    } else {
      loc.split.children[loc.index] = home.index == 0
          ? DockTabs([made, tile as DockPane])
          : DockTabs([tile as DockPane, made], active: 1);
    }
    return;
  }
  final axis = home.at == DropPosition.left || home.at == DropPosition.right
      ? DockAxis.horizontal
      : DockAxis.vertical;
  final before = home.at == DropPosition.left || home.at == DropPosition.above;
  final share = home.share.clamp(0.05, 0.9);
  final path = _pathTo(root, home.beside);
  if (path != null &&
      home.depth < path.length &&
      path[home.depth].split.axis == axis) {
    final (:split, :index) = path[home.depth];
    final total = split.shares.fold<double>(0, (a, b) => a + b);
    final at = before ? index : index + 1;
    split.children.insert(at, made);
    split.shares.insert(at, total * share / (1 - share));
    return;
  }
  if (loc.split.axis == axis) {
    final whole = loc.split.shares[loc.index];
    loc.split.shares[loc.index] = whole * (1 - share);
    final at = before ? loc.index : loc.index + 1;
    loc.split.children.insert(at, made);
    loc.split.shares.insert(at, whole * share);
  } else {
    loc.split.children[loc.index] = DockSplit(
      axis,
      before ? [made, loc.tile] : [loc.tile, made],
      before ? [share, 1 - share] : [1 - share, share],
    );
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
