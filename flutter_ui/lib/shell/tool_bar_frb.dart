// The toolbar: the strip of tools under the menu bar (docs/07 §1.7).
//
// **In plain terms.** This is the row every editor has under its menus — the
// arrow, the hand, the pen — and picking one of them says what dragging in the
// Viewer will do. Tools that do the same sort of job share a button the way
// After Effects shares them: the button shows the one you last used, and
// holding it (or right-clicking) opens the rest. Pressing the tool's key does
// the same thing without the flyout, and pressing it again steps through the
// group.
//
// **What it does not do.** It arms a tool; it does not perform one. The armed
// tool is one value on [ToolsState] that panels read — today the Viewer changes
// its cursor from it and nothing else, because the drawing, painting and puppet
// behaviours are not built. That is deliberate: the whole tool set is specified
// (docs/07 §1.7) and shipping the strip with the unbuilt ones missing would
// leave no place to put them and no way to see what is coming. A tool that
// changes nothing yet says so in its tooltip.
//
// The right-hand end carries what the shell has nowhere else to put: the
// workspace strip §1.4 asks for.
//
// **Two positions.** Settings, Interface, Toolbar position is Top or Left
// (docs/design-alt/15-DESIGN-LANTERN.md §12B.1). Top is this strip. Left puts
// the thirteen tools and the magnet on [LumitToolRailFrb], a 44px column down
// the window's left edge, and the top line takes the tool options and the
// workspace strip ([LumitTopLineToolsFrb]): a rail has no room for a number
// field or six words, and a strip kept above the dock for those two alone
// would add a row to the column the rail already adds. Every key, flyout and
// chord stays in one widget tree.
//
// **The magnet is back**. It sat here once, was taken off when nothing
// in the application read it — a toggle that changes nothing is worse
// than a missing one — and returns now that the Viewer's layer drags reach for
// the guides and the grid with it. Docs/07 §4.5 puts the switch that governs a
// gesture on the toolbar; the Viewer's own guides menu shows the same switch
// under its own name.

import 'dart:math' show max, min;

import 'package:flutter/rendering.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart'
    show BridgeBrushShape, BridgeLayerKind;
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../main.dart';
import '../panels/viewer_paint.dart' show brushShapeLabel;
import '../state/dock.dart';
import '../state/settings.dart' show ToolBarPosition;
import '../state/tools.dart';
import '../theme/motion.dart';
import '../theme/style_sheet.dart';
import '../theme/theme.dart';
import '../widgets/colour_picker.dart';
import '../panels/timeline_extras_frb.dart' show showMenuAt;
import '../widgets/controls.dart';
import 'menu_bar_frb.dart'
    show MenuEntry, showMenuEntries, workspaceEntryRows;
import 'settings_window_frb.dart' show SettingsPage, showSettingsWindowFrb;
import 'window_frame.dart';

/// How tall the strip is, and how wide and tall one tool button is.
///
/// 15-DESIGN §7.2 puts toolbar controls on the household's full ≥44px hit
/// extent. A button keeps that **across** — 44 wide, which is what makes the
/// row easy to hit along its length and is the spacing the strip is read by —
/// and gives it up down the page: the strip runs the full width of the
/// window, so a 44px-tall band of mostly empty space is a stripe of chrome
/// taken off the panels underneath for nothing. 28 keeps the 16px icon at its
/// §5 floor with room to breathe.
const double toolBarHeight = 30;
const double _toolButtonWidth = 44;
const double _toolButtonHeight = 28;

/// How wide the rail is when the toolbar stands on the left: the full 44 the
/// strip gives up down the page.
const double toolRailWidth = 44;

/// The strip's height under [shape]. Studio keeps its 30; Desk draws it at 32
/// on the module; Lantern's is a 44 band in the room with 32 pills on it.
double toolBarHeightFor(ThemeShape shape) => switch (shape) {
      ThemeShape.studio => toolBarHeight,
      ThemeShape.desk => 32,
      ThemeShape.lantern => 44,
    };

/// Lantern's pills: the tool and options pills on the strip, and the workspace
/// pill, which is shorter because it also rides the 40 band under Left.
const double _lanternPill = 32;
const double _workspacePill = 28;

/// The last of the simple pointer tools. The rail draws a seam after it, and
/// so does the strip once it is grouped into pills.
const ToolGroup _seamAfter = ToolGroup.razor;

/// The tool groups in the order the strip lists them: the pointer tools first,
/// then the ones that draw, then the ones that paint, then the camera — After
/// Effects' own grouping, which is the order the audience already knows.
const List<ToolGroup> toolBarOrder = [
  ToolGroup.select,
  ToolGroup.hand,
  ToolGroup.zoom,
  ToolGroup.rotate,
  ToolGroup.anchor,
  ToolGroup.razor,
  ToolGroup.shape,
  ToolGroup.shape3d,
  ToolGroup.pen,
  ToolGroup.type,
  ToolGroup.paint,
  ToolGroup.roto,
  ToolGroup.puppet,
  ToolGroup.camera,
];

/// The keymap action that arms each group, for the tooltips' shortcut text.
String _actionFor(ToolGroup group) =>
    toolActions.entries.firstWhere((e) => e.value == group).key;

/// One group's button on the strip, and its row in the overflow menu once
/// folded. Selection and the armed group never fold.
FoldItem _foldedTool(LumitTheme t, ToolGroup group, ToolsState tools,
    {required bool seam, bool rail = false}) {
  final member = tools.memberOf(group);
  final armed = tools.tool.group == group;
  final enabled =
      ToolMode.builtMembersOf(group).isNotEmpty && tools.arms(group);
  return FoldItem(
    keep: armed || group == ToolGroup.select,
    label: member.label,
    icon: lumitIcon(member.icon,
        size: iconSize,
        color: !enabled
            ? t.textDisabled
            : armed
                ? t.accent
                : t.textSecondary),
    selected: armed,
    onPressed: enabled ? () => tools.select(member) : null,
    child: Flex(
      direction: rail ? Axis.vertical : Axis.horizontal,
      mainAxisSize: MainAxisSize.min,
      children: [
        _ToolButton(group: group, tools: tools, rail: rail),
        if (seam && group == _seamAfter) _ToolBarDivider(vertical: rail),
      ],
    ),
  );
}

class LumitToolBarFrb extends StatelessWidget {
  const LumitToolBarFrb({super.key});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = context.watch<LumitUiState>();
    // Lantern groups the row into three pills standing in the room; Studio
    // and Desk keep the flat strip, welded under the menu bar by a hairline.
    final pills = t.tokens.roomed;
    // Named, so a style can dress the toolbar's buttons apart from others.
    return StylePart(name: 'toolbar', child: GestureDetector(
      onSecondaryTapUp: (d) =>
          showMenuEntries(context, d.globalPosition, _barRows(ui)),
      child: Container(
      height: toolBarHeightFor(t.shape),
      decoration: BoxDecoration(
        color: pills
            ? t.room
            : t.shape == ThemeShape.desk
                ? t.surface1
                : t.surface2,
        border: pills ? null : Border(bottom: BorderSide(color: t.hairline)),
      ),
      child: ListenableBuilder(
        // The model too: the axes only show once the comp has something
        // in depth.
        listenable: Listenable.merge([ui.tools, ui.model]),
        builder: (context, _) {
          final options = _toolOptions(t, ui, height: _lanternPill);
          // The tool pill keeps the magnet at its end; the flat strip keeps
          // the magnet after the options, where it has always stood.
          final tools = Row(
            key: const ValueKey('tool-bar-tools'),
            mainAxisSize: MainAxisSize.min,
            children: [
              // Folds from the end into the overflow mark on a narrow
              // window, keeping Selection and the tool in your hand.
              Flexible(
                child: FoldingRow(
                  markKey: const ValueKey('tool-bar-overflow'),
                  markWidth: _toolButtonWidth,
                  markHeight: _toolButtonHeight,
                  items: [
                    for (final group in toolBarOrder)
                      _foldedTool(t, group, ui.tools, seam: pills),
                  ],
                ),
              ),
              if (pills) ...[
                const _ToolBarDivider(),
                _SnapButton(tools: ui.tools),
              ],
            ],
          );
          return Row(
            children: [
              const SizedBox(width: 4),
              // The tools and their options take the whole left-hand end, so
              // the workspace strip is held against the *right* edge where
              // docs/07 §1.4 puts it. Expanded rather than letting the two
              // scroll views size themselves: a loose Flexible only takes the
              // width it needs, which left the workspace buttons sitting
              // immediately beside the last tool with the free space stranded
              // past them.
              Expanded(
                child: _ToolsFirst(children: [
                  _pill(t, 'tool-pill', tools, height: _lanternPill),
                  if (options != null) ...[_gap(pills), options],
                ]),
              ),
              if (!pills) ...[
                const _ToolBarDivider(),
                _SnapButton(tools: ui.tools),
              ],
              _gap(pills),
              _workspaces(t),
              const SizedBox(width: 6),
            ],
          );
        },
      ),
    )));
  }
}

/// What the top line carries while the tools stand on the rail: the armed
/// tool's options after a seam, and the workspace strip at the right end. The
/// strip above the dock is not mounted then, so the rail adds a column without
/// the strip still taking its row.
class LumitTopLineToolsFrb extends StatelessWidget {
  const LumitTopLineToolsFrb({super.key});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = context.watch<LumitUiState>();
    final pills = t.tokens.roomed;
    return ListenableBuilder(
      listenable: Listenable.merge([ui.tools, ui.model]),
      builder: (context, _) {
        final options = _toolOptions(t, ui, height: _workspacePill);
        // The workspaces take their own width at the right end, where
        // docs/07 §1.4 puts them, and the options take what is left. The
        // strip only scrolls once the half is too narrow for it, not by a
        // share of the width; the 22 is the gaps and the end inset.
        return LayoutBuilder(
          builder: (context, c) => Row(
            children: [
              // The options stand right-aligned beside the workspaces, so
              // the two groups read as one block at the line's end.
              // What the options leave empty moves the window, where this
              // line is the window's top (window_frame.dart).
              if (options != null) ...[
                _gap(pills),
                Expanded(
                  child: Stack(children: [
                    const Positioned.fill(child: WindowDragArea()),
                    Align(alignment: AlignmentDirectional.centerEnd, child: options),
                  ]),
                ),
              ] else
                const Expanded(child: WindowDragArea()),
              _gap(pills),
              ConstrainedBox(
                constraints: BoxConstraints(maxWidth: c.maxWidth - 22),
                child: SingleChildScrollView(
                  scrollDirection: Axis.horizontal,
                  child: _workspaces(t),
                ),
              ),
              const SizedBox(width: 6),
            ],
          ),
        );
      },
    );
  }
}

/// The bar's own menu, on its empty stretch and on the rail: where the tools
/// stand, which Settings also has. Option rows, so the positions can be
/// compared without reopening it.
List<MenuEntry> _barRows(LumitUiState ui) {
  final settings = ui.workspace.interface;
  return [
    MenuEntry.submenu(l10n.settingsToolBarPosition, [
      for (final (position, label) in [
        (ToolBarPosition.auto, l10n.styleChoice),
        (ToolBarPosition.top, l10n.toolBarTop),
        (ToolBarPosition.left, l10n.toolBarLeft),
      ])
        MenuEntry.option(
          label,
          () {
            settings.toolBarPosition = position;
            ui.workspace.settingsChanged();
          },
          checked: () => settings.toolBarPosition == position,
        ),
    ]),
  ];
}

/// What stands between two groups on the row: air between pills, a hairline
/// on the flat strip.
Widget _gap(bool pills) =>
    pills ? const SizedBox(width: 8) : const _ToolBarDivider();

/// The tools, then the gap and the armed tool's options. The tools take the
/// width they need first, so none folds while the bar has room for it. The
/// options take what is left, and keep at least a third of the row (or
/// their own width, if less) so a narrow window still shows some of them.
class _ToolsFirst extends MultiChildRenderObjectWidget {
  const _ToolsFirst({required super.children});

  @override
  RenderObject createRenderObject(BuildContext context) => _RenderToolsFirst();
}

class _RenderToolsFirst extends RenderBox
    with
        ContainerRenderObjectMixin<RenderBox, MultiChildLayoutParentData>,
        RenderBoxContainerDefaultsMixin<RenderBox, MultiChildLayoutParentData> {
  @override
  void setupParentData(RenderBox child) {
    if (child.parentData is! MultiChildLayoutParentData) {
      child.parentData = MultiChildLayoutParentData();
    }
  }

  @override
  void performLayout() {
    final room = constraints.maxWidth;
    BoxConstraints upTo(double w) =>
        BoxConstraints(maxWidth: max(0.0, w), maxHeight: constraints.maxHeight);
    final tools = firstChild!;
    final gap = childAfter(tools);
    final options = gap == null ? null : childAfter(gap);
    if (gap == null || options == null) {
      tools.layout(upTo(room), parentUsesSize: true);
    } else {
      gap.layout(upTo(room), parentUsesSize: true);
      final free = room - gap.size.width;
      // Each at the most it could have, to learn what it wants.
      tools.layout(upTo(free), parentUsesSize: true);
      options.layout(upTo(free), parentUsesSize: true);
      final left = free - tools.size.width;
      final width = min(options.size.width, max(left, room / 3));
      tools.layout(upTo(free - width), parentUsesSize: true);
      options.layout(upTo(width), parentUsesSize: true);
    }
    var height = 0.0;
    for (var c = firstChild; c != null; c = childAfter(c)) {
      height = max(height, c.size.height);
    }
    height = constraints.constrainHeight(height);
    var x = 0.0;
    for (var c = firstChild; c != null; c = childAfter(c)) {
      (c.parentData! as MultiChildLayoutParentData).offset =
          Offset(x, (height - c.size.height) / 2);
      x += c.size.width;
    }
    size = constraints.constrain(Size(room, height));
  }

  @override
  void paint(PaintingContext context, Offset offset) =>
      defaultPaint(context, offset);

  @override
  bool hitTestChildren(BoxHitTestResult result, {required Offset position}) =>
      defaultHitTestChildren(result, position: position);
}

/// The armed tool's own options, when it has any: After Effects puts them
/// beside the tools, and the strip is empty for the tools that draw nothing.
/// Lantern shows no pill then either: one that only said "no options" was a
/// thing to read with nothing in it.
Widget? _toolOptions(LumitTheme t, LumitUiState ui, {required double height}) {
  final tools = ui.tools;
  final shows = toolOptionsFor(tools.tool,
      inDepth: ui.model.heldLayers.any((entry) =>
          entry.info.switches.threeD ||
          entry.info.kind == BridgeLayerKind.light ||
          entry.info.kind == BridgeLayerKind.camera));
  if (shows == ToolOptions.none) return null;
  // The row is centred on the pill's height, as the workspace strip's are
  // on theirs; a bare child takes the top of the box it is given. The width
  // stays the content's, so the flat strip's row is laid out as it was.
  return _pill(
    t,
    'tool-options-pill',
    Align(
      alignment: AlignmentDirectional.centerStart,
      widthFactor: 1,
      child: SingleChildScrollView(
        scrollDirection: Axis.horizontal,
        child: _ToolOptions(tools: tools, shows: shows),
      ),
    ),
    height: height,
    padding: const EdgeInsets.symmetric(horizontal: 8),
  );
}

/// The workspace strip, as a 28 pill under Lantern whose padding is the inset
/// the fronted capsule keeps from its edge on every side.
Widget _workspaces(LumitTheme t) => _pill(
      t,
      'workspace-pill',
      const _WorkspaceStrip(),
      height: _workspacePill,
      padding: EdgeInsets.all(t.tokens.pillInset),
    );

/// A capsule on the room colour, under Lantern; [child] untouched otherwise.
/// The fill is surface1 with the card's own shadow, so the three read as the
/// same kind of object as the panes below them.
Widget _pill(
  LumitTheme t,
  String key,
  Widget child, {
  double? height,
  EdgeInsets padding = const EdgeInsets.symmetric(horizontal: 4),
}) {
  if (!t.tokens.roomed) return child;
  return Container(
    key: ValueKey<String>(key),
    height: height,
    padding: padding,
    decoration: BoxDecoration(
      color: t.surface1,
      borderRadius: BorderRadius.circular(ShapeTokens.stadium),
      boxShadow: t.tokens.cardShadow,
    ),
    child: child,
  );
}

/// The toolbar as a rail (docs/design-alt/15-DESIGN-DESK.md §12B.1): the
/// thirteen groups in a 44px column, a seam after the six pointer tools, and
/// the magnet at the foot. The shell mounts it beside the dock while the
/// position is Left, and the top line takes the options and the workspaces.
class LumitToolRailFrb extends StatelessWidget {
  const LumitToolRailFrb({super.key});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = context.watch<LumitUiState>();
    final pills = t.tokens.roomed;
    return GestureDetector(
      onSecondaryTapUp: (d) =>
          showMenuEntries(context, d.globalPosition, _barRows(ui)),
      child: Container(
      width: toolRailWidth,
      decoration: BoxDecoration(
        color: pills
            ? t.room
            : t.shape == ThemeShape.desk
                ? t.surface1
                : t.surface2,
        border: pills ? null : Border(right: BorderSide(color: t.hairline)),
      ),
      // Under Lantern the pill stands level with the cards' top and bottom;
      // the rail itself stays flush to the window edge.
      padding: EdgeInsets.symmetric(vertical: t.tokens.windowInset),
      child: ListenableBuilder(
        listenable: ui.tools,
        builder: (context, _) => _pill(
          t,
          'tool-pill',
          Column(
            key: const ValueKey('tool-bar-tools'),
            children: [
              // A short window folds the last tools into the mark at the
              // foot rather than cutting one in half.
              Expanded(
                child: Align(
                  alignment: Alignment.topCenter,
                  child: FoldingRow(
                    axis: Axis.vertical,
                    markKey: const ValueKey('tool-bar-overflow'),
                    markWidth: _toolButtonWidth,
                    markHeight:
                        t.shape == ThemeShape.desk ? toolRailWidth : 36,
                    items: [
                      for (final group in toolBarOrder)
                        _foldedTool(t, group, ui.tools,
                            seam: true, rail: true),
                    ],
                  ),
                ),
              ),
              const _ToolBarDivider(vertical: true),
              _SnapButton(tools: ui.tools),
              const SizedBox(height: 6),
            ],
          ),
        ),
      ),
    ));
  }
}

/// **The magnet** (docs/07 §4.5): whether a drag on the picture reaches
/// for the guides and the grid.
///
/// Dressed as the graph editor's and the Timeline's are — lit reads as the
/// glyph at foreground strength on the button's own face, off as a frameless
/// muted mark. No accent: §3.1's list of that colour's jobs is closed, and a
/// magnet is not on it.
class _SnapButton extends StatelessWidget {
  final ToolsState tools;
  const _SnapButton({required this.tools});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final on = tools.snapping;
    return LumitTooltip(
      message: on ? l10n.tipSnapOn : l10n.tipSnapOff,
      child: GestureDetector(
        // What the magnet reaches for, as View ▸ has it.
        onSecondaryTapUp: (d) => showMenuEntries(context, d.globalPosition, [
          MenuEntry.toggle(
            l10n.viewerOverlaySnap,
            () => tools.snapping = !tools.snapping,
            checked: () => tools.snapping,
          ),
          MenuEntry.toggle(
            l10n.menuSnapToGrid,
            () => tools.snapToGrid = !tools.snapToGrid,
            action: 'viewer.snap.grid.toggle',
            checked: () => tools.snapToGrid,
          ),
          MenuEntry.toggle(
            l10n.menuSnapToLayers,
            () => tools.snapToLayers = !tools.snapToLayers,
            checked: () => tools.snapToLayers,
          ),
        ]),
        child: HouseButton(
          key: const ValueKey('tool-snapping'),
          small: true,
          frameless: !on,
          padding: const EdgeInsets.symmetric(horizontal: 4),
          onPressed: () => tools.snapping = !on,
          child: lumitIcon(LumitIcon.magnet,
              size: iconSize, color: on ? t.textPrimary : t.textMuted),
        ),
      ),
    );
  }
}

/// Which options the armed tool puts on the strip.
enum ToolOptions {
  none,

  /// Fill and size: what the Type tool sets a new line in.
  type,

  /// Fill and stroke: what a shape tool draws with. Both live since shape
  /// layers landed — a shape layer's art carries a fill colour, a
  /// stroke colour and a stroke width, and a width of zero draws no outline.
  shape,

  /// The brush: the colour it lays down, and its size, hardness and opacity.
  /// All four live — painting is built.
  paint,

  /// The puppet mesh: **Density** and **Expansion**. No fill —
  /// a pin lays no colour down, it takes hold of pixels that are already there.
  puppet,

  /// The roto scribble: **Size**, and nothing else. No fill and no
  /// hardness — a roto stroke lays no colour down and has no edge to soften; it
  /// says "this is the subject" over the pixels it covers, and how wide it
  /// covers is the whole of what there is to set.
  roto,

  /// Which way a 3D layer's arrows and rings point: Local, World or View,
  /// while the comp has a 3D layer, a light or a camera.
  axes,

  /// The camera tools: what an orbit swings round.
  camera,
}

/// The options [tool] shows on the toolbar.
ToolOptions toolOptionsFor(ToolMode tool, {bool inDepth = false}) =>
    switch (tool.group) {
      ToolGroup.type => ToolOptions.type,
      ToolGroup.paint => ToolOptions.paint,
      ToolGroup.shape || ToolGroup.pen => ToolOptions.shape,
      ToolGroup.puppet => ToolOptions.puppet,
      ToolGroup.roto => ToolOptions.roto,
      ToolGroup.select || ToolGroup.rotate when inDepth => ToolOptions.axes,
      ToolGroup.camera => ToolOptions.camera,
      _ => ToolOptions.none,
    };

/// The fill, size and stroke controls: After Effects' tool options area.
class _ToolOptions extends StatelessWidget {
  final ToolsState tools;
  final ToolOptions shows;

  const _ToolOptions({required this.tools, required this.shows});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    if (shows == ToolOptions.axes) {
      return Row(
        children: [
          for (final (axes, icon, tip) in [
            (GizmoAxes.local, LumitIcon.localAxes, l10n.tipLocalAxes),
            (GizmoAxes.world, LumitIcon.worldAxes, l10n.tipWorldAxes),
            (GizmoAxes.view, LumitIcon.viewAxes, l10n.tipViewAxes),
          ])
            LumitTooltip(
              message: tip,
              // Lit as the magnet is, the one in use at full strength.
              child: HouseButton(
                key: ValueKey<String>('tool-axes-${axes.name}'),
                small: true,
                frameless: tools.axes != axes,
                padding: const EdgeInsets.symmetric(horizontal: 4),
                onPressed: () => tools.axes = axes,
                child: lumitIcon(icon,
                    size: iconSize,
                    color: tools.axes == axes ? t.textPrimary : t.textMuted),
              ),
            ),
          // What the Rotation tool turns, as After Effects' Set menu does.
          if (tools.tool.group == ToolGroup.rotate) ...[
            const _ToolBarDivider(),
            for (final (sets, label, tip) in [
              (
                RotateSets.orientation,
                l10n.transformOrientation,
                l10n.tipRotateSetsOrientation
              ),
              (
                RotateSets.rotation,
                l10n.transformRotation,
                l10n.tipRotateSetsRotation
              ),
            ])
              LumitTooltip(
                message: tip,
                child: HouseButton(
                  key: ValueKey<String>('tool-rotate-sets-${sets.name}'),
                  small: true,
                  frameless: tools.rotateSets != sets,
                  padding: const EdgeInsets.symmetric(horizontal: 6),
                  onPressed: () => tools.rotateSets = sets,
                  child: Text(label,
                      style: t.small.copyWith(
                          color: tools.rotateSets == sets
                              ? t.textPrimary
                              : t.textMuted)),
                ),
              ),
          ],
        ],
      );
    }
    // The scribble's width, on its own, for the mesh's reason: a roto stroke
    // lays no colour down either.
    if (shows == ToolOptions.roto) {
      return Row(
        children: [
          _Number(
            label: l10n.toolSize,
            tip: l10n.tipRotoSize,
            value: tools.rotoSize,
            min: 1,
            max: 2000,
            suffix: ' px',
            onChanged: (v) => tools.rotoSize = v,
          ),
        ],
      );
    }
    if (shows == ToolOptions.camera) {
      return Row(
        children: [
          Text(l10n.toolOrbitAround,
              style: t.small.copyWith(color: t.textSecondary)),
          const SizedBox(width: 5),
          BareDropdown<CameraPivot>(
            key: const ValueKey<String>('tool-camera-pivot'),
            value: tools.cameraPivot,
            options: CameraPivot.values,
            label: (pivot) => pivot.label,
            onChanged: (v) => tools.cameraPivot = v,
          ),
        ],
      );
    }
    // The mesh, on its own: a pin lays no colour down, so the fill swatch every
    // other drawing tool opens with would be a control governing nothing.
    if (shows == ToolOptions.puppet) {
      return Row(
        children: [
          _Number(
            label: l10n.toolPuppetDensity,
            tip: l10n.tipPuppetDensity,
            value: tools.puppetDensity,
            min: 2,
            max: 500,
            suffix: ' px',
            onChanged: (v) => tools.puppetDensity = v,
          ),
          _Number(
            label: l10n.toolPuppetExpansion,
            tip: l10n.tipPuppetExpansion,
            value: tools.puppetExpansion,
            min: 0,
            max: 100,
            suffix: ' px',
            onChanged: (v) => tools.puppetExpansion = v,
          ),
        ],
      );
    }
    return Row(
      children: [
        _Swatch(
          label: l10n.toolFill,
          colour: tools.fill,
          resetTo: ToolColour.white,
          onPicked: (colour) => tools.fill = colour,
          // None is for shapes: text and paint always lay a colour down.
          on: shows != ToolOptions.shape || tools.fillOn,
          onToggle: shows == ToolOptions.shape
              ? (on) => tools.fillOn = on
              : null,
        ),
        const SizedBox(width: 6),
        if (shows == ToolOptions.paint) ...[
          _Number(
            label: l10n.toolSize,
            tip: l10n.tipBrushSize,
            value: tools.brushSize,
            min: 1,
            max: 2000,
            suffix: ' px',
            onChanged: (v) => tools.brushSize = v,
          ),
          Padding(
            padding: const EdgeInsetsDirectional.only(end: 6),
            child: LumitTooltip(
              message: l10n.tipBrushShape,
              child: Row(
                children: [
                  Text(l10n.toolShape,
                      style: t.small.copyWith(color: t.textSecondary)),
                  const SizedBox(width: 5),
                  BareDropdown<BridgeBrushShape>(
                    key: const ValueKey<String>('tool-brush-shape'),
                    value: tools.brushShape,
                    options: BridgeBrushShape.values,
                    label: brushShapeLabel,
                    onChanged: (v) => tools.brushShape = v,
                  ),
                ],
              ),
            ),
          ),
          _Number(
            label: l10n.toolHardness,
            tip: l10n.tipEdgeHardness,
            value: tools.brushHardness,
            min: 0,
            max: 100,
            suffix: '%',
            onChanged: (v) => tools.brushHardness = v,
          ),
          Padding(
            padding: const EdgeInsetsDirectional.only(end: 6),
            child: LumitTooltip(
              message: l10n.tipBrushPressure,
              child: Row(
                children: [
                  HouseCheckbox(
                    key: const ValueKey<String>('tool-brush-pressure'),
                    value: tools.brushPressureSize,
                    onChanged: (v) => tools.brushPressureSize = v,
                  ),
                  const SizedBox(width: 4),
                  Text(l10n.toolPressureSize,
                      style: t.small.copyWith(color: t.textSecondary)),
                ],
              ),
            ),
          ),
          _Number(
            label: l10n.toolOpacity,
            tip: l10n.tipMarkOpacity,
            value: tools.brushOpacity,
            min: 0,
            max: 100,
            suffix: '%',
            onChanged: (v) => tools.brushOpacity = v,
          ),
        ] else if (shows == ToolOptions.type)
          SizedBox(
            width: 62,
            child: LumitTooltip(
              message: l10n.tipTextSize,
              child: DragValueField(
                value: tools.textSize,
                min: 1,
                max: 2000,
                suffix: ' px',
                onChanged: (v) => tools.textSize = v.toDouble(),
              ),
            ),
          )
        else ...[
          _Swatch(
            label: l10n.toolStroke,
            colour: tools.stroke,
            resetTo: ToolColour.black,
            onPicked: (colour) => tools.stroke = colour,
            on: shows != ToolOptions.shape || tools.strokeOn,
            onToggle: shows == ToolOptions.shape
                ? (on) => tools.strokeOn = on
                : null,
          ),
          const SizedBox(width: 6),
          _Number(
            label: l10n.toolWidth,
            tip: l10n.tipOutlineWidth,
            value: tools.strokeWidth,
            min: 0,
            max: 1000,
            suffix: ' px',
            onChanged: (v) => tools.strokeWidth = v,
          ),
        ],
      ],
    );
  }
}

/// A labelled number on the options strip.
class _Number extends StatelessWidget {
  final String label;
  final String tip;
  final double value;
  final double min;
  final double max;
  final String suffix;
  final ValueChanged<double> onChanged;

  const _Number({
    required this.label,
    required this.tip,
    required this.value,
    required this.min,
    required this.max,
    required this.suffix,
    required this.onChanged,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Padding(
      padding: const EdgeInsetsDirectional.only(end: 6),
      child: LumitTooltip(
        message: tip,
        child: Row(
          children: [
            Text(label, style: t.small.copyWith(color: t.textSecondary)),
            const SizedBox(width: 5),
            SizedBox(
              width: 58,
              child: DragValueField(
                value: value,
                min: min,
                max: max,
                suffix: suffix,
                onChanged: (v) => onChanged(v.toDouble()),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// A colour well that opens the picker. [onPicked] null draws it inert.
///
/// With [onToggle] the label is a control of its own, in the accent: a click
/// on it picks Solid colour or None, as After Effects' Fill and Stroke labels
/// do, and a well that is off wears the None slash.
class _Swatch extends StatelessWidget {
  final String label;
  final ToolColour colour;
  final ValueChanged<ToolColour>? onPicked;
  final bool on;
  final ValueChanged<bool>? onToggle;

  /// What the well's menu resets to.
  final ToolColour resetTo;

  const _Swatch({
    required this.label,
    required this.colour,
    required this.onPicked,
    this.on = true,
    this.onToggle,
    required this.resetTo,
  });

  Widget _label(LumitTheme t) {
    final toggle = onToggle;
    if (toggle == null) {
      return Text(label, style: t.small.copyWith(color: t.textSecondary));
    }
    Widget row(String key, String text, bool value, void Function(void) close) =>
        MenuRow(
          key: ValueKey<String>('tool-swatch-$key'),
          selected: on == value,
          onPressed: () {
            close(null);
            toggle(value);
          },
          child: Text(text),
        );
    return LumitTooltip(
      message: l10n.tipSwatchKind(label),
      child: HoverMark(
        builder: (context, lit) => GestureDetector(
          onTapUp: (details) => showMenuAt<void>(
            context: context,
            position: details.globalPosition,
            rows: (close) => [
              row('solid', l10n.toolSolidColour, true, close),
              row('none', l10n.none, false, close),
            ],
          ),
          child: Text(label,
              style: t.small.copyWith(
                  color: Color.lerp(t.accent, t.accentHover, lit))),
        ),
      ),
    );
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    Widget well(double lit) => Container(
          width: 20,
          height: 20,
          decoration: BoxDecoration(
            color: on
                // Shown as the Viewer shows it: the colour is linear light.
                ? PickedColour(colour.r, colour.g, colour.b).onScreen.clipped
                : t.surface1,
            border: Border.all(
                color:
                    Color.lerp(t.hairlineStrong, t.textPrimary, 0.7 * lit)!),
            borderRadius: BorderRadius.circular(3),
          ),
          child: on ? null : CustomPaint(painter: _NoneSlash(t.error)),
        );
    // With a label of its own to click, the well alone opens the picker.
    final split = onToggle != null;
    Widget row(double lit) => Row(
          children: [
            _label(t),
            const SizedBox(width: 5),
            well(lit),
          ],
        );
    if (onPicked == null) return row(0);
    final picker = LumitTooltip(
      message: l10n.tipSwatchColour(label),
      child: HoverMark(
        builder: (context, lit) => GestureDetector(
          // The menu every colour well has.
          onSecondaryTapUp: (details) => showColourWellMenu(
            context,
            details.globalPosition,
            current: PickedColour(colour.r, colour.g, colour.b),
            resetTo: PickedColour(resetTo.r, resetTo.g, resetTo.b),
            linear: true,
            onPicked: (picked) =>
                onPicked!(ToolColour(picked.r, picked.g, picked.b)),
          ),
          onTapUp: (details) => showColourPicker(
            context: context,
            position: details.globalPosition,
            initial: PickedColour(colour.r, colour.g, colour.b),
            scale: ColourScale.bytes,
            linear: true,
            onCommit: (picked) {
              onPicked!(ToolColour(picked.r, picked.g, picked.b));
              // Picking a colour is asking for one.
              onToggle?.call(true);
            },
          ),
          child: split ? well(lit) : row(lit),
        ),
      ),
    );
    if (!split) return picker;
    return Row(
      children: [
        _label(t),
        const SizedBox(width: 5),
        picker,
      ],
    );
  }
}

/// The red diagonal a None well wears, as in After Effects.
class _NoneSlash extends CustomPainter {
  final Color colour;
  const _NoneSlash(this.colour);

  @override
  void paint(Canvas canvas, Size size) => canvas.drawLine(
        Offset(size.width - 2, 2),
        Offset(2, size.height - 2),
        Paint()
          ..color = colour
          ..strokeWidth = 1.5,
      );

  @override
  bool shouldRepaint(_NoneSlash old) => old.colour != colour;
}

/// A hairline seam on the strip, turned on its side for the rail.
class _ToolBarDivider extends StatelessWidget {
  final bool vertical;
  const _ToolBarDivider({this.vertical = false});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      width: vertical ? 20 : 1,
      height: vertical ? 1 : 20,
      margin: vertical
          ? const EdgeInsets.symmetric(vertical: 6)
          : const EdgeInsets.symmetric(horizontal: 6),
      color: t.hairline,
    );
  }
}

/// One group's button: the member it stands for, armed by a click, with the
/// rest of the group behind a press-and-hold or a right-click.
class _ToolButton extends StatefulWidget {
  final ToolGroup group;
  final ToolsState tools;

  /// Standing on the rail: a taller cell, and the flyout opens to the right.
  final bool rail;

  const _ToolButton({
    required this.group,
    required this.tools,
    this.rail = false,
  });

  @override
  State<_ToolButton> createState() => _ToolButtonState();
}

class _ToolButtonState extends State<_ToolButton> {
  bool _hover = false;

  /// Whether this tool was armed at the last build, so a switch of tool can
  /// be told from a hover.
  bool? _wasActive;

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final member = widget.tools.memberOf(widget.group);
    final active = widget.tools.tool.group == widget.group;
    // Only the built ones: the flyout leaves the rest out, so a group with one
    // working tool is a plain button with no corner mark.
    final members = ToolMode.builtMembersOf(widget.group);
    // A group nothing in which is built is on the strip but cannot be pressed:
    // the tool set is the specification, and a button that visibly
    // cannot be pressed says "coming" where a missing one says nothing.
    // So is one this machine may not arm, because it can only look.
    final enabled = members.isNotEmpty && widget.tools.arms(widget.group);

    // 15-DESIGN §5's icon states, exactly: secondary at rest, primary on hover,
    // accent when this is the tool in your hand — and muted for a group that
    // cannot be armed at all. Lantern fills the armed button with the accent
    // instead, so its glyph takes the colour a primary button's label does.
    final filled = t.tokens.roomed;
    final desk = t.shape == ThemeShape.desk;
    final colour = !enabled
        ? t.textDisabled
        : active
            ? (filled ? t.surface0 : t.accent)
            : _hover
                ? t.textPrimary
                : t.textSecondary;
    // Studio tints the armed button; Desk gives it no fill and a 2px accent
    // index on the bar's outer edge, the top of a strip button and the left
    // of a rail one. Lantern leaves the cell bare and fills a disc inside it,
    // drawn below, so the cell's own fill is never used there.
    final Color? fill = filled
        ? null
        : active
            ? (desk ? null : t.accent.withValues(alpha: 0.16))
            : _hover
                ? t.surface4
                : null;
    // Lantern's disc: the pill's 32 less the inset either side, so on the
    // strip it sits 3 in from the pill's edge, and the rail draws the same one.
    final disc = _lanternPill - 2 * t.tokens.pillInset;
    final index = BorderSide(color: t.accent, width: 2);
    final decoration = desk
        ? BoxDecoration(
            color: fill,
            border: active
                ? (widget.rail ? Border(left: index) : Border(top: index))
                : null,
          )
        : BoxDecoration(
            color: fill,
            borderRadius: BorderRadius.circular(t.tokens.actionRadius),
          );
    // The rail's cell: the full 44 on Desk's module, 36 in Lantern's pill.
    final height = !widget.rail
        ? _toolButtonHeight
        : desk
            ? toolRailWidth
            : 36.0;
    // Arming a tool is a key press, many times an hour, so the mark moves
    // at once, on the tool armed and the one let go. The hover light moves
    // as a hover does, and the glyph goes with the fill behind it.
    final switched = _wasActive != null && _wasActive != active;
    _wasActive = active;
    final step = active || switched
        ? MotionSpec.still
        : _hover
            ? scope.motion.hoverIn
            : scope.motion.hoverOut;

    return LumitTooltip(
      message: _tooltip(member),
      action: _actionFor(widget.group),
      child: MouseRegion(
        cursor: enabled ? SystemMouseCursors.click : SystemMouseCursors.basic,
        onEnter: (_) => setState(() => _hover = enabled),
        onExit: (_) => setState(() => _hover = false),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: enabled ? () => widget.tools.select(member) : null,
          // A press-and-hold opens the group's flyout, as it always has.
          // The right button opens the button's own menu: the group, and
          // what else there is to do with the tool.
          onLongPress:
              enabled && members.length > 1 ? () => _openFlyout(context) : null,
          onSecondaryTapUp: (d) => _openMenu(context, d.globalPosition),
          child: AnimatedContainer(
            key: ValueKey<String>('tool-${widget.group.name}'),
            duration: step.duration,
            curve: step.curve,
            width: _toolButtonWidth,
            height: height,
            decoration: decoration,
            child: Stack(
              children: [
                if (filled)
                  Center(
                    child: AnimatedContainer(
                      key: ValueKey<String>('tool-disc-${widget.group.name}'),
                      duration: step.duration,
                      curve: step.curve,
                      width: disc,
                      height: disc,
                      decoration: BoxDecoration(
                        shape: BoxShape.circle,
                        color: active
                            ? t.accent
                            : _hover
                                ? t.surface4
                                : null,
                      ),
                    ),
                  ),
                Center(
                  child: TweenAnimationBuilder<Color?>(
                    tween: ColorTween(end: colour),
                    duration: step.duration,
                    curve: step.curve,
                    builder: (context, ink, _) => lumitIcon(member.icon,
                        size: iconSize, color: ink ?? colour),
                  ),
                ),
                // The corner mark that says there is more under this button —
                // the same promise After Effects' little triangle makes.
                if (members.length > 1)
                  Positioned(
                    right: 4,
                    bottom: 4,
                    child: CustomPaint(
                      size: const Size(4, 4),
                      painter: _FlyoutMarkPainter(colour),
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }

  /// The tooltip: the tool's name and, for a tool whose behaviour is not
  /// built, the plain fact that arming it changes nothing yet. Saying so is
  /// cheaper than a user discovering it by dragging and getting silence. The
  /// shortcut is the tooltip's own to add, from the group's keymap action,
  /// straight after the name. The "Hold" that used to follow a group's name
  /// is gone: beside the chord it read as "hold Q", and the corner mark says
  /// there is more.
  String _tooltip(ToolMode member) => member.ready
      ? member.label
      : '${member.label} · ${l10n.tipNotBuiltYet}';

  /// The button's menu: every tool in the group, the one in hand ticked,
  /// then the options' reset where the tool has options, and the way to
  /// its key. One list for every button, so a lone tool has a menu too.
  void _openMenu(BuildContext context, Offset at) {
    final tools = widget.tools;
    final face = tools.memberOf(widget.group);
    final armed = tools.tool;
    final hasOptions = toolOptionsFor(face, inDepth: true) != ToolOptions.none;
    showMenuEntries(context, at, [
      for (final member in ToolMode.builtMembersOf(widget.group))
        MenuEntry(
          member.label,
          member.ready && tools.arms(widget.group)
              ? () => tools.select(member)
              : null,
          // The key arms the member on the button's face, so it is that row
          // that shows it.
          action: member == face ? _actionFor(widget.group) : null,
          checked: member == armed,
          icon: member.icon,
        ),
      MenuEntry.divider(),
      if (hasOptions)
        MenuEntry(l10n.menuResetToolOptions,
            () => tools.resetOptionsOf(widget.group)),
      MenuEntry(
          l10n.menuChangeShortcut,
          () => showSettingsWindowFrb(context,
              initialPage: SettingsPage.shortcuts,
              row: _actionFor(widget.group))),
    ]);
  }

  void _openFlyout(BuildContext context) {
    final box = context.findRenderObject();
    if (box is! RenderBox) return;
    // Under the button on the strip, to the right of it on the rail.
    final origin = box.localToGlobal(
        widget.rail ? Offset(box.size.width, 0) : Offset(0, box.size.height));
    final tools = widget.tools;
    showLumitPopup<ToolMode>(
      context: context,
      position: origin,
      builder: (close) => _ToolFlyout(
        group: widget.group,
        armed: tools.tool,
        onPick: (tool) {
          close(tool);
          tools.select(tool);
        },
      ),
    );
  }
}

/// The hidden tools under a group button.
class _ToolFlyout extends StatelessWidget {
  final ToolGroup group;
  final ToolMode armed;
  final ValueChanged<ToolMode> onPick;

  const _ToolFlyout({
    required this.group,
    required this.armed,
    required this.onPick,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return FloatSurface(
      width: 210,
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          // The tools not built yet are left out. They come back by themselves
          // the day [ToolMode.ready] says they work.
          for (final member in ToolMode.builtMembersOf(group))
            MenuRow(
              key: ValueKey<String>('tool-flyout-${member.name}'),
              selected: member == armed,
              // A member that is not built is listed and does nothing when
              // clicked — the same rule the buttons follow.
              enabled: member.ready,
              onPressed: () => onPick(member),
              child: Row(
                children: [
                  lumitIcon(member.icon,
                      size: iconSize,
                      color: !member.ready
                          ? t.textDisabled
                          : member == armed
                              ? t.accent
                              : t.textSecondary),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      member.label,
                      style: member.ready
                          ? null
                          : TextStyle(color: t.textDisabled),
                    ),
                  ),
                  if (!member.ready)
                    Text(l10n.notBuilt,
                        style: t.small.copyWith(color: t.textDisabled)),
                ],
              ),
            ),
        ],
      ),
    );
  }
}

/// The little triangle in a group button's corner.
class _FlyoutMarkPainter extends CustomPainter {
  final Color colour;
  const _FlyoutMarkPainter(this.colour);

  @override
  void paint(Canvas canvas, Size size) {
    final path = Path()
      ..moveTo(size.width, 0)
      ..lineTo(size.width, size.height)
      ..lineTo(0, size.height)
      ..close();
    canvas.drawPath(path, Paint()..color = colour);
  }

  @override
  bool shouldRepaint(_FlyoutMarkPainter old) => old.colour != colour;
}

/// An arrangement's shape, leaving out what changes as it is used (the tab in
/// front, the stacked panels open), to tell when it has moved from a preset.
String _arrangement(DockNode node) => switch (node) {
      DockPane(:final panel) => panel.name,
      DockTabs(:final children) => '[${children.map(_arrangement).join(',')}]',
      DockSplit(:final axis, :final children, :final shares) => () {
          final total = shares.fold<double>(0, (a, b) => a + b);
          return '${axis.name}(${[
            for (var i = 0; i < children.length; i++)
              '${(shares[i] / total).toStringAsFixed(2)} '
                  '${_arrangement(children[i])}',
          ].join(',')})';
        }(),
    };

/// The workspace switcher docs/07 §1.4 requires in the window chrome: the
/// shipped presets as mono-caps kickers, the current one ticked in the accent.
class _WorkspaceStrip extends StatelessWidget {
  const _WorkspaceStrip();

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = context.watch<LumitUiState>();
    final app = context.read<LumitState>();
    final active = ui.workspace.activePreset;
    // The preset in force wears the settings' dot once it has been moved
    // from how it ships, and its menu's Reset workspace puts it back.
    final moved = active != null &&
        _arrangement(ui.workspace.dock) != _arrangement(presetLayout(active));
    final row = Row(
      key: const ValueKey('workspace-strip'),
      mainAxisSize: MainAxisSize.min,
      children: [
        for (final (i, preset) in WorkspacePreset.values.indexed)
          _StripEntry(
            key: ValueKey<String>('workspace-${preset.name}'),
            label: preset.title,
            slot: i + 1,
            active: preset == active,
            moved: preset == active && moved,
            onPressed: () => ui.workspace.applyWorkspacePreset(preset),
            onMenu: (context, at) => showMenuEntries(
                context,
                at,
                workspaceEntryRows(context, app, ui,
                    preset: preset, slot: i + 1)),
          ),
        // The user's own, after the presets and in the same order the chords
        // count (docs/07 §1.4). Drawn by exactly the same rules — a workspace
        // somebody saved is a workspace, not a lesser kind of one.
        for (final (i, saved) in ui.workspace.userWorkspaces.indexed)
          _StripEntry(
            key: ValueKey<String>('workspace-user-${saved.name}'),
            label: saved.name,
            slot: WorkspacePreset.values.length + i + 1,
            active: saved.name == ui.workspace.activeUserWorkspace,
            onPressed: () => ui.workspace.applyUserWorkspace(saved.name),
            onMenu: (context, at) => showMenuEntries(
                context,
                at,
                workspaceEntryRows(context, app, ui,
                    user: saved.name,
                    slot: WorkspacePreset.values.length + i + 1)),
          ),
      ],
    );
    // Lantern's pill already makes the names a set of places. On the flat
    // strips they sat as bare words beside the menu headings, so they stand
    // in a track of their own, the fronted one filled.
    if (t.tokens.roomed) return row;
    return Container(
      key: const ValueKey('workspace-track'),
      padding: const EdgeInsets.all(2),
      decoration: BoxDecoration(
        color: t.shape == ThemeShape.desk ? t.surface2 : t.surface0,
        borderRadius: BorderRadius.circular(t.tokens.actionRadius),
        border: Border.all(color: t.hairline),
      ),
      child: row,
    );
  }
}

/// One name on the workspace strip — a shipped preset or one of the user's
/// own, drawn identically because they are the same kind of thing.
///
/// Studio: mono-caps kickers with an **accent tick under the one in force**
/// (docs/15 §12A.1, §3.1: the workspace tabs are what "the active tab tick"
/// means). The word itself stays grey: the tick is the state, so the strip
/// reads as names with one underlined rather than as one coloured word.
/// Desk: lowercase words in the body face, the fronted one over a 2px accent
/// rule, the top line of its mockup. Lantern: the fronted name is the
/// accent-filled capsule of a segmented pill, and there is no tick to draw
/// under a fill.
///
/// The padding is what fits the name into the strip, and the strip is 14px
/// shorter than it was: at 12 above and below, 24px of padding in a
/// 30px band left the words with five and they were squeezed out of sight,
/// leaving a button that could be pressed and not read.
class _StripEntry extends StatelessWidget {
  final String label;

  /// Where it stands on the strip, from one: the slot its chord counts by.
  final int slot;
  final bool active;
  final VoidCallback onPressed;

  /// Moved from how its preset ships: the settings' dot in its corner.
  final bool moved;

  /// Opens the name's own menu at a right-click.
  final void Function(BuildContext context, Offset at) onMenu;

  const _StripEntry({
    super.key,
    required this.label,
    required this.slot,
    required this.active,
    required this.onPressed,
    required this.onMenu,
    this.moved = false,
  });

  /// The name on one line, cut short past what a strip can give one of a
  /// user's own, so a long name never pushes the rest off the bar.
  Widget _name(String text, TextStyle style) => ConstrainedBox(
        constraints: const BoxConstraints(maxWidth: 140),
        child: Text(text,
            style: style, maxLines: 1, overflow: TextOverflow.ellipsis),
      );

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final Widget button = switch (t.shape) {
      // The capsule is the pill's 28 less the inset either side. Height 1.0
      // gives the line its font size and no leading, so the button centring
      // the box centres the word to the pixel.
      ThemeShape.lantern => SizedBox(
          height: _workspacePill - 2 * t.tokens.pillInset,
          child: HouseButton(
            frameless: true,
            active: active,
            padding: const EdgeInsets.symmetric(horizontal: 10),
            onPressed: onPressed,
            // The word's optical centre sits above its box's, so it is
            // nudged down by a pixel to look centred. Two of padding is all
            // the fronted capsule has room for, any more squeezes the line
            // and drops that word below the others.
            child: Padding(
              padding: const EdgeInsets.only(top: 2),
              child: _name(
                label,
                t.body.copyWith(
                  fontSize: 12,
                  height: 1.0,
                  // A style that dresses the fronted button names its ink.
                  color: active
                      ? partStyle(context, t, 'button', states: const {'on'})
                              .colour ??
                          accentInk(t)
                      : t.textMuted,
                ),
              ),
            ),
          ),
        ),
      ThemeShape.desk => HouseButton(
          frameless: true,
          padding: const EdgeInsets.fromLTRB(9, 2, 9, 0),
          onPressed: onPressed,
          // The rule is painted over the foot of the padding, so it takes no
          // room of its own and the fronted word stays level with the rest.
          child: Container(
            padding: const EdgeInsets.only(bottom: 4),
            foregroundDecoration: active
                ? BoxDecoration(
                    border: Border(
                        bottom: BorderSide(color: t.accent, width: 2)))
                : null,
            child: _name(
              t.kickerCase(label),
              t.body.copyWith(color: active ? t.textPrimary : t.textMuted),
            ),
          ),
        ),
      // A segment of the track: the fronted one wears the button's own
      // active fill, so the strip reads as one control with one choice made.
      ThemeShape.studio => HouseButton(
          frameless: true,
          active: active,
          padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 3),
          onPressed: onPressed,
          child: _name(t.kickerCase(label), active ? t.kickerOn : t.kicker),
        ),
    };
    // The name and the chord that switches to it. A slot past the ninth has
    // no chord, and the tooltip then shows the name alone.
    return LumitTooltip(
      message: label,
      action: 'workspace.switch.$slot',
      child: GestureDetector(
        onSecondaryTapUp: (d) => onMenu(context, d.globalPosition),
        child: Stack(children: [
          button,
          if (moved)
            PositionedDirectional(
              top: 2,
              end: 2,
              child: Container(
                key: const ValueKey('workspace-moved'),
                width: 4,
                height: 4,
                decoration: BoxDecoration(
                  // On Lantern's accent capsule, in the capsule's ink.
                  color: t.shape == ThemeShape.lantern ? accentInk(t) : t.accent,
                  shape: BoxShape.circle,
                ),
              ),
            ),
        ]),
      ),
    );
  }
}

/// The pointer the Viewer shows while [tool] is armed.
///
/// The one place the armed tool changes anything today, and it is worth having
/// on its own: a cursor is how a toolbar tells you it is listening, and it
/// costs nothing to be honest about which tools are only a cursor so far.
MouseCursor viewerCursorFor(ToolMode tool) => switch (tool) {
      // The Hand and the Zoom draw their own over the picture: Windows
      // has no grab or magnifier pointer, and Flutter's names for them fall
      // back to the plain arrow there. Their own layers hide the system one and
      // paint it; this is what shows underneath.
      ToolMode.hand || ToolMode.zoom => SystemMouseCursors.none,
      // Nothing over the *picture*: the razor cuts in the Timeline, where it
      // draws its own blade. A crosshair here promised a precision the Viewer
      // has no razor gesture to spend.
      ToolMode.razor => SystemMouseCursors.basic,
      ToolMode.anchor => SystemMouseCursors.move,
      // Type points at where the words will start; horizontal takes the
      // system's I-beam, and vertical has one drawn for it over the picture
      // because no platform ships a sideways beam.
      ToolMode.typeHorizontal => SystemMouseCursors.text,
      ToolMode.typeVertical => SystemMouseCursors.none,
      // The tools that aim at a pixel keep the hardware crosshair: the
      // OS moves it at input rate whatever the application's frame rate is
      // doing, and their overlays ask for the same pointer, adding only
      // decoration beside it — the badge, the brush ring.
      _ => tool.group == ToolGroup.shape ||
              tool.group == ToolGroup.shape3d ||
              tool.group == ToolGroup.pen ||
              tool.group == ToolGroup.paint ||
              tool.group == ToolGroup.roto ||
              tool.group == ToolGroup.puppet ||
              tool.group == ToolGroup.camera
          ? SystemMouseCursors.precise
          : SystemMouseCursors.basic,
    };
