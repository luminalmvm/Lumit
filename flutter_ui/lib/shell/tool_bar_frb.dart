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
import 'package:lumit_flutter/src/rust/api/effect.dart' show BridgeScalar_Static;
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../main.dart';
import '../panels/shape_target.dart';
import '../panels/viewer_paint.dart' show brushShapeLabel;
import '../state/dock.dart';
import '../state/preview_throttle.dart';
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

const double toolRailWidth = 44;

double toolBarHeightFor(ThemeShape shape) => switch (shape) {
      ThemeShape.studio => toolBarHeight,
      ThemeShape.desk => 32,
      ThemeShape.lantern => 44,
    };

const double _lanternPill = 32;
const double _workspacePill = 28;

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

String _actionFor(ToolGroup group) =>
    toolActions.entries.firstWhere((e) => e.value == group).key;

/// One group's button on the strip, and its row in the overflow menu once
/// folded. Selection and the armed group never fold.
FoldItem _foldedTool(LumitTheme t, ToolGroup group, ToolsState tools,
    {required bool seam,
    bool rail = false,
    bool inDepth = false,
    bool beside = false,
    bool held = false}) {
  final member = tools.memberOf(group);
  final armed = tools.tool.group == group;
  final enabled =
      ToolMode.builtMembersOf(group).isNotEmpty && tools.arms(group);
  return FoldItem(
    keep: armed || held || group == ToolGroup.select,
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
        _ToolItem(
            group: group,
            tools: tools,
            inDepth: inDepth,
            beside: beside && !rail,
            held: held,
            rail: rail),
        if (seam && group == _seamAfter) _ToolBarDivider(vertical: rail),
      ],
    ),
  );
}

bool _inDepth(LumitUiState ui) => ui.model.heldLayers.any((entry) =>
    entry.info.switches.threeD ||
    entry.info.kind == BridgeLayerKind.light ||
    entry.info.kind == BridgeLayerKind.camera);

class LumitToolBarFrb extends StatelessWidget {
  const LumitToolBarFrb({super.key});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = context.watch<LumitUiState>();
    final pills = t.tokens.roomed;
    // Named, so a style can dress the toolbar's buttons apart from others.
    return StylePart(name: 'toolbar', child: GestureDetector(
      onSecondaryTapUp: (d) =>
          showMenuEntries(context, d.globalPosition, _barRows(ui, top: true)),
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
        listenable: Listenable.merge([ui.tools, ui.model, ui.workspace]),
        builder: (context, _) {
          final inDepth = _inDepth(ui);
          final beside = ui.workspace.interface.toolOptionsBeside;
          final options =
              beside ? null : _toolOptions(t, ui, height: _lanternPill);
          final tools = Row(
            key: const ValueKey('tool-bar-tools'),
            mainAxisSize: MainAxisSize.min,
            children: [
              Flexible(
                child: _ToolStrip(
                  active: ui.tools.tool.group,
                  builder: (held) => FoldingRow(
                    markKey: const ValueKey('tool-bar-overflow'),
                    markWidth: _toolButtonWidth,
                    markHeight: _toolButtonHeight,
                    items: [
                      for (final group in toolBarOrder)
                        _foldedTool(t, group, ui.tools,
                            seam: pills,
                            inDepth: inDepth,
                            beside: beside,
                            held: beside && held.contains(group)),
                    ],
                  ),
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

List<MenuEntry> _barRows(LumitUiState ui, {bool top = false}) {
  final settings = ui.workspace.interface;
  return [
    if (top)
      MenuEntry.toggle(
        l10n.settingsToolOptionsBeside,
        () {
          settings.toolOptionsBeside = !settings.toolOptionsBeside;
          ui.workspace.settingsChanged();
        },
        checked: () => settings.toolOptionsBeside,
      ),
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
  final shows = toolOptionsFor(tools.tool, inDepth: _inDepth(ui));
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
        child: _ToolOptions(
            tools: tools, shows: shows, group: tools.tool.group),
      ),
    ),
    height: height,
    padding: const EdgeInsets.symmetric(horizontal: 8),
  );
}

Widget _workspaces(LumitTheme t) => _pill(
      t,
      'workspace-pill',
      const _WorkspaceStrip(),
      height: _workspacePill,
      padding: EdgeInsets.all(t.tokens.pillInset),
    );

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
                  child: _ToolStrip(
                    active: ui.tools.tool.group,
                    axis: Axis.vertical,
                    builder: (_) => FoldingRow(
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

enum ToolOptions {
  none,

  type,

  /// Fill and stroke: what a shape tool draws with. Both live since shape
  /// layers landed — a shape layer's art carries a fill colour, a
  /// stroke colour and a stroke width, and a width of zero draws no outline.
  shape,

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

  camera,
}

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

class _ToolOptions extends StatelessWidget {
  final ToolsState tools;
  final ToolOptions shows;
  final ToolGroup group;

  const _ToolOptions(
      {required this.tools, required this.shows, required this.group});

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
          if (group == ToolGroup.rotate) ...[
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
    if (shows == ToolOptions.shape) {
      return _ShapeOptions(tools: tools, group: group);
    }
    final paint = tools.paintOf(group);
    return Row(
      children: [
        // No None here: text and paint always lay a colour down.
        _Swatch(
          label: l10n.toolFill,
          colour: paint.fill,
          resetTo: ToolColour.white,
          onPicked: (colour) => tools.setPaint(group, fill: colour),
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
          ),
      ],
    );
  }
}

class _ShapeOptions extends StatefulWidget {
  final ToolsState tools;
  final ToolGroup group;

  const _ShapeOptions({required this.tools, required this.group});

  @override
  State<_ShapeOptions> createState() => _ShapeOptionsState();
}

class _ShapeOptionsState extends State<_ShapeOptions> {
  final PreviewThrottle _throttle = PreviewThrottle();

  double? _width;

  late LumitState _app;

  @override
  void dispose() {
    _throttle.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    _app = Provider.of<LumitState>(context, listen: false);
    return ListenableBuilder(
      listenable: Listenable.merge(
          [widget.tools, ui.model, ui.selectedLayers, ui.selectedProperties]),
      builder: (context, _) {
        final targets = shapeTargets(ui);
        final comp = ui.selectedComp;
        if (targets.isEmpty || comp == null) {
          _width = null;
          return _row(ui, const [], paintOfTools(widget.tools.paintOf(widget.group)));
        }
        final shape = shownShape(targets.first);
        Widget live() => _row(ui, targets,
            paintOfShape(shape, widget.tools.paintOf(widget.group), comp,
                ui.playheadFrame.value));
        final moves = [
          for (final colour in [shape.fill, shape.stroke])
            if (colour != null) ...[colour.r, colour.g, colour.b],
          shape.strokeWidth,
        ].any((s) => s is! BridgeScalar_Static);
        return moves
            ? ValueListenableBuilder<int>(
                valueListenable: ui.playheadFrame,
                builder: (context, _, __) => live(),
              )
            : live();
      },
    );
  }

  void _commit(LumitUiState ui, List<ShapeTarget> targets, ShapeEdit edit) {
    _settle();
    if (!commitShapes(_app, ui, targets, edit)) {
      previewShapes(ui, targets.first, null);
    }
  }

  void _preview(LumitUiState ui, ShapeTarget target, ShapeEdit? edit) =>
      _throttle.request(() => previewShapes(ui, target, edit));

  void _drop(LumitUiState ui, ShapeTarget target) {
    _settle();
    previewShapes(ui, target, null);
  }

  void _settle() {
    _throttle.cancel();
    if (mounted && _width != null) setState(() => _width = null);
  }

  Widget _row(LumitUiState ui, List<ShapeTarget> targets, ShapePaint paint) {
    final tools = widget.tools;
    final group = widget.group;
    final target = targets.firstOrNull;
    return Row(
      children: [
        _Swatch(
          label: l10n.toolFill,
          colour: paint.fill,
          resetTo: ToolColour.white,
          onPicked: (colour) => target == null
              ? tools.setPaint(group, fill: colour)
              : _commit(ui, targets, fillColourEdit(colour)),
          onPreview: target == null
              ? null
              : (colour) => _preview(ui, target, fillColourEdit(colour)),
          onClosed: target == null ? null : () => _drop(ui, target),
          on: paint.fillOn,
          onToggle: (on) => target == null
              ? tools.setPaint(group, fillOn: on)
              : _commit(ui, targets, fillOnEdit(on, paint.fill)),
        ),
        const SizedBox(width: 6),
        _Swatch(
          label: l10n.toolStroke,
          colour: paint.stroke,
          resetTo: ToolColour.black,
          onPicked: (colour) => target == null
              ? tools.setPaint(group, stroke: colour)
              : _commit(ui, targets, strokeColourEdit(colour)),
          onPreview: target == null
              ? null
              : (colour) => _preview(ui, target, strokeColourEdit(colour)),
          onClosed: target == null ? null : () => _drop(ui, target),
          on: paint.strokeOn,
          onToggle: (on) => target == null
              ? tools.setPaint(group, strokeOn: on)
              : _commit(ui, targets, strokeOnEdit(on, paint.stroke)),
        ),
        const SizedBox(width: 6),
        _Number(
          label: l10n.toolWidth,
          tip: l10n.tipOutlineWidth,
          value: _width ?? paint.width,
          min: 0,
          max: 1000,
          suffix: ' px',
          onChanged: (v) => target == null
              ? tools.setPaint(group, strokeWidth: v)
              : _commit(ui, targets, strokeWidthEdit(v, paint.stroke)),
          onChangeLive: target == null
              ? null
              : (v) {
                  setState(() => _width = v);
                  _preview(ui, target, strokeWidthEdit(v, paint.stroke));
                },
          onDragCancel: target == null ? null : () => _drop(ui, target),
        ),
      ],
    );
  }
}

class _Number extends StatelessWidget {
  final String label;
  final String tip;
  final double value;
  final double min;
  final double max;
  final String suffix;
  final ValueChanged<double> onChanged;

  final ValueChanged<double>? onChangeLive;
  final VoidCallback? onDragCancel;

  const _Number({
    required this.label,
    required this.tip,
    required this.value,
    required this.min,
    required this.max,
    required this.suffix,
    required this.onChanged,
    this.onChangeLive,
    this.onDragCancel,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final live = onChangeLive;
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
                onChangeLive: live == null ? null : (v) => live(v.toDouble()),
                onDragCancel: onDragCancel,
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

  final ValueChanged<ToolColour>? onPreview;
  final VoidCallback? onClosed;

  final ToolColour resetTo;

  const _Swatch({
    required this.label,
    required this.colour,
    required this.onPicked,
    this.on = true,
    this.onToggle,
    this.onPreview,
    this.onClosed,
    required this.resetTo,
  });

  Future<void> _pick(BuildContext context, Offset at) async {
    final opened = PickedColour(colour.r, colour.g, colour.b);
    final preview = onPreview;
    var written = false;
    await showColourPicker(
      context: context,
      position: at,
      initial: opened,
      scale: ColourScale.bytes,
      linear: true,
      onPreview: preview == null
          ? null
          : (picked) => preview(ToolColour(picked.r, picked.g, picked.b)),
      onCommit: (picked) {
        if (preview != null &&
            !written &&
            (picked.r - opened.r).abs() < 1e-4 &&
            (picked.g - opened.g).abs() < 1e-4 &&
            (picked.b - opened.b).abs() < 1e-4) {
          return;
        }
        written = true;
        onPicked!(ToolColour(picked.r, picked.g, picked.b));
        onToggle?.call(true);
      },
    );
    onClosed?.call();
  }

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
          onSecondaryTapUp: (details) => showColourWellMenu(
            context,
            details.globalPosition,
            current: PickedColour(colour.r, colour.g, colour.b),
            resetTo: PickedColour(resetTo.r, resetTo.g, resetTo.b),
            linear: true,
            onPicked: (picked) =>
                onPicked!(ToolColour(picked.r, picked.g, picked.b)),
          ),
          onTapUp: (details) => _pick(context, details.globalPosition),
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

class _StripScope extends InheritedWidget {
  final _Slide slide;

  final double room;

  const _StripScope({
    required this.slide,
    required this.room,
    required super.child,
  });

  static _StripScope of(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<_StripScope>()!;

  @override
  bool updateShouldNotify(_StripScope old) => room != old.room;
}

class _ToolStrip extends StatefulWidget {
  final ToolGroup active;
  final Axis axis;
  final Widget Function(Set<ToolGroup> held) builder;

  const _ToolStrip({
    required this.active,
    required this.builder,
    this.axis = Axis.horizontal,
  });

  @override
  State<_ToolStrip> createState() => _ToolStripState();
}

class _ToolStripState extends State<_ToolStrip>
    with SingleTickerProviderStateMixin {
  late final _Slide _slide = _Slide(this)..active = widget.active;
  bool _hovered = false;
  final Set<ToolGroup> _held = {};

  void _tune() {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    _slide
      ..shape = t.shape
      ..rail = widget.axis == Axis.vertical
      ..disc = _lanternPill - 2 * t.tokens.pillInset
      ..spec = scope.motion.slide
      ..lead = scope.motion.slideLead;
  }

  @override
  void didUpdateWidget(_ToolStrip old) {
    super.didUpdateWidget(old);
    if (widget.active == old.active) return;
    if (_hovered) _held.add(old.active);
    _tune();
    _slide.moveTo(widget.active);
  }

  @override
  void dispose() {
    _slide.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    _tune();
    final t = ThemeScope.of(context).theme;
    return LayoutBuilder(
      builder: (context, c) {
        final fixed = _toolButtonWidth * (toolBarOrder.length + 1) + 14;
        final room = widget.axis == Axis.vertical
            ? 0.0
            : max(c.maxWidth / 3, c.maxWidth - fixed);
        return _StripScope(
          slide: _slide,
          room: room,
          child: MouseRegion(
            onEnter: (_) => _hovered = true,
            onExit: (_) {
              _hovered = false;
              if (_held.isEmpty) return;
              setState(_held.clear);
              _slide.follow();
            },
            child: Stack(
              key: _slide.stripKey,
              children: [
                Positioned.fill(
                  child: IgnorePointer(
                    child: CustomPaint(painter: _TrayPainter(_slide, t)),
                  ),
                ),
                widget.builder(_held),
              ],
            ),
          ),
        );
      },
    );
  }
}

class _Slide extends ChangeNotifier {
  _Slide(TickerProvider vsync)
      : controller = AnimationController(vsync: vsync) {
    controller
      ..addListener(notifyListeners)
      ..addStatusListener((_) => notifyListeners());
  }

  final AnimationController controller;
  final GlobalKey stripKey = GlobalKey();
  final Map<ToolGroup, GlobalKey> _cells = {};

  ToolGroup? active;
  ToolGroup? _prev;
  ThemeShape shape = ThemeShape.studio;
  bool rail = false;
  double disc = 26;
  Curve lead = Curves.easeOutCubic;
  MotionSpec _spec = MotionSpec.still;
  Rect? _from;

  set spec(MotionSpec spec) {
    _spec = spec;
    controller.duration = spec.duration;
  }

  GlobalKey cellKey(ToolGroup group) =>
      _cells.putIfAbsent(group, () => GlobalKey());

  RenderBox? _box(GlobalKey key) {
    final box = key.currentContext?.findRenderObject();
    return box is RenderBox && box.attached && box.hasSize ? box : null;
  }

  bool get fades => shape == ThemeShape.desk;

  double get shown => fades && controller.value < 1
      ? _spec.curve.transform(controller.value)
      : 1;

  Rect? _target(ToolGroup? group) {
    final strip = _box(stripKey);
    final item = group == null ? null : _box(cellKey(group));
    if (strip == null || item == null) return null;
    final rect = item.localToGlobal(Offset.zero, ancestor: strip) & item.size;
    if (shape != ThemeShape.lantern) return rect;
    final cell =
        Rect.fromLTWH(rect.left, rect.top, _toolButtonWidth, rect.height);
    final head =
        Rect.fromCenter(center: cell.center, width: disc, height: disc);
    final inset = (_toolButtonWidth - disc) / 2;
    final grown = rect.right - cell.right;
    final right = rect.right -
        inset * (1 - (grown / (2 * inset)).clamp(0.0, 1.0).toDouble());
    return Rect.fromLTRB(
        head.left, head.top, max(head.right, right), head.bottom);
  }

  Rect? tray() {
    final to = _target(active);
    final from = _from;
    final v = controller.value;
    if (to == null) return from;
    if (fades || from == null || v >= 1) return to;
    final behind = _spec.curve.transform(v);
    final ahead = lead.transform(v);
    final onward = rail ? to.top >= from.top : to.left >= from.left;
    double mix(double a, double b, double k) => a + (b - a) * k;
    final far = onward ? ahead : behind;
    final near = onward ? behind : ahead;
    return rail
        ? Rect.fromLTRB(
            mix(from.left, to.left, behind),
            mix(from.top, to.top, near),
            mix(from.right, to.right, behind),
            mix(from.bottom, to.bottom, far))
        : Rect.fromLTRB(
            mix(from.left, to.left, near),
            mix(from.top, to.top, behind),
            mix(from.right, to.right, far),
            mix(from.bottom, to.bottom, behind));
  }

  Rect ink(Rect tray) => shape == ThemeShape.lantern
      ? Rect.fromLTWH(tray.left, tray.top, disc, disc)
      : tray;

  Rect? faded() => !fades || controller.value >= 1 || _prev == null
      ? null
      : _target(_prev) ?? _from;

  void moveTo(ToolGroup group) {
    if (group == active) return;
    _from = tray();
    _prev = active;
    active = group;
    if (_from != null) controller.forward(from: 0);
  }

  void follow() {
    if (controller.isAnimating) return;
    _from = null;
    _prev = null;
    controller.forward(from: 0);
  }

  Shader mask(BuildContext context, Rect bounds, Color colour) {
    final box = context.findRenderObject();
    final strip = _box(stripKey);
    final tray = strip == null ? null : this.tray();
    final old = strip == null ? null : faded();
    final clear = colour.withValues(alpha: 0);
    final length = rail ? bounds.height : bounds.width;
    var lo = 0.0;
    var hi = 0.0;
    var lit = colour;
    if (strip != null && box is RenderBox && box.attached) {
      final at = box.localToGlobal(Offset.zero, ancestor: strip);
      for (final (rect, weight) in [
        if (tray != null) (ink(tray), shown),
        if (old != null) (old, 1 - shown),
      ]) {
        final covered = rect.shift(-at);
        lo = ((rail ? covered.top : covered.left) / length)
            .clamp(0.0, 1.0)
            .toDouble();
        hi = ((rail ? covered.bottom : covered.right) / length)
            .clamp(0.0, 1.0)
            .toDouble();
        if (hi > lo) {
          lit = colour.withValues(alpha: colour.a * weight);
          break;
        }
      }
    }
    final none = hi <= lo;
    return LinearGradient(
      begin: rail ? Alignment.topCenter : Alignment.centerLeft,
      end: rail ? Alignment.bottomCenter : Alignment.centerRight,
      colors: none ? [clear, clear] : [clear, clear, lit, lit, clear, clear],
      stops: none ? null : [0, lo, lo, hi, hi, 1],
    ).createShader(bounds);
  }

  @override
  void dispose() {
    controller.dispose();
    super.dispose();
  }
}

class _TrayPainter extends CustomPainter {
  final _Slide slide;
  final LumitTheme t;

  _TrayPainter(this.slide, this.t) : super(repaint: slide);

  @override
  void paint(Canvas canvas, Size size) {
    final tray = slide.tray();
    if (tray == null) return;
    switch (t.shape) {
      case ThemeShape.studio:
        canvas.drawRRect(
          RRect.fromRectAndRadius(
              tray, Radius.circular(t.tokens.actionRadius)),
          Paint()..color = t.accent.withValues(alpha: 0.16),
        );
      case ThemeShape.desk:
        void rule(Rect at, double weight) => canvas.drawRect(
              slide.rail
                  ? Rect.fromLTWH(at.left, at.top, 2, at.height)
                  : Rect.fromLTWH(at.left, at.top, at.width, 2),
              Paint()..color = t.accent.withValues(alpha: t.accent.a * weight),
            );
        rule(tray, slide.shown);
        final old = slide.faded();
        if (old != null) rule(old, 1 - slide.shown);
      case ThemeShape.lantern:
        canvas.drawRRect(
          RRect.fromRectAndRadius(tray, Radius.circular(tray.height / 2)),
          Paint()..color = t.accent.withValues(alpha: 0.18),
        );
        canvas.drawOval(slide.ink(tray), Paint()..color = t.accent);
    }
  }

  @override
  bool shouldRepaint(_TrayPainter old) => old.t != t || old.slide != slide;
}

class _Inked extends StatelessWidget {
  final _Slide slide;
  final Color ink;
  final Widget child;

  const _Inked({required this.slide, required this.ink, required this.child});

  @override
  Widget build(BuildContext context) => AnimatedBuilder(
        animation: slide,
        child: child,
        builder: (context, face) => ShaderMask(
          blendMode: BlendMode.srcATop,
          shaderCallback: (bounds) => slide.mask(context, bounds, ink),
          child: face,
        ),
      );
}

class _TrayClip extends CustomClipper<Rect> {
  final _Slide slide;
  final GlobalKey key;

  _TrayClip(this.slide, this.key) : super(reclip: slide);

  @override
  Rect getClip(Size size) {
    final whole = Offset.zero & size;
    final box = key.currentContext?.findRenderObject();
    final strip = slide._box(slide.stripKey);
    final tray = slide.tray();
    if (tray == null || strip == null || box is! RenderBox || !box.attached) {
      return whole;
    }
    final at = box.localToGlobal(Offset.zero, ancestor: strip);
    final shown = whole.intersect(tray.shift(-at));
    return shown.isEmpty ? Rect.zero : shown;
  }

  @override
  bool shouldReclip(_TrayClip old) => old.slide != slide || old.key != key;
}

class _ToolItem extends StatelessWidget {
  final ToolGroup group;
  final ToolsState tools;
  final bool rail;
  final bool inDepth;
  final bool beside;
  final bool held;

  const _ToolItem({
    required this.group,
    required this.tools,
    required this.rail,
    required this.inDepth,
    required this.beside,
    required this.held,
  });

  @override
  Widget build(BuildContext context) {
    final slide = _StripScope.of(context).slide;
    final face = tools.memberOf(group);
    final button = _ToolButton(group: group, tools: tools, rail: rail);
    if (!beside || toolOptionsFor(face, inDepth: true) == ToolOptions.none) {
      return KeyedSubtree(key: slide.cellKey(group), child: button);
    }
    final shows = toolOptionsFor(face, inDepth: inDepth);
    return Row(
      key: slide.cellKey(group),
      mainAxisSize: MainAxisSize.min,
      children: [
        button,
        _Pocket(
          group: group,
          tools: tools,
          shows: shows,
          open: tools.tool.group == group && shows != ToolOptions.none,
          held: held && shows != ToolOptions.none,
        ),
      ],
    );
  }
}

class _Pocket extends StatefulWidget {
  final ToolGroup group;
  final ToolsState tools;
  final ToolOptions shows;
  final bool open;
  final bool held;

  const _Pocket({
    required this.group,
    required this.tools,
    required this.shows,
    required this.open,
    required this.held,
  });

  @override
  State<_Pocket> createState() => _PocketState();
}

class _PocketState extends State<_Pocket> {
  ToolOptions? _kind;
  final GlobalKey _clip = GlobalKey();
  bool _shown = false;
  bool _opening = false;

  @override
  Widget build(BuildContext context) {
    final spec = ThemeScope.of(context).motion.slide;
    final scope = _StripScope.of(context);
    final room = scope.room;
    if (widget.shows != ToolOptions.none) _kind = widget.shows;
    final kind = _kind;
    _opening = widget.open && (_opening || !_shown);
    _shown = widget.open || widget.held;
    return TweenAnimationBuilder<double>(
      tween: Tween<double>(end: _shown ? 1 : 0),
      duration: spec.duration,
      curve: spec.curve,
      child: kind == null
          ? null
          : ConstrainedBox(
              constraints:
                  BoxConstraints(maxWidth: room, maxHeight: _toolButtonHeight),
              child: SingleChildScrollView(
                scrollDirection: Axis.horizontal,
                child: Padding(
                  padding: const EdgeInsetsDirectional.only(start: 2, end: 6),
                  child: _ToolOptions(
                      tools: widget.tools, shows: kind, group: widget.group),
                ),
              ),
            ),
      builder: (context, p, body) => p <= 0 || body == null
          ? const SizedBox.shrink()
          : ClipRect(
              key: _clip,
              clipper: _opening ? _TrayClip(scope.slide, _clip) : null,
              child: Align(
                alignment: AlignmentDirectional.centerStart,
                widthFactor: p,
                child: body,
              ),
            ),
    );
  }
}

class _ToolButton extends StatefulWidget {
  final ToolGroup group;
  final ToolsState tools;

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

    // 15-DESIGN §5's icon states: secondary at rest, primary on hover, muted
    // for a group that cannot be armed. The armed look is the highlight's.
    final filled = t.tokens.roomed;
    final desk = t.shape == ThemeShape.desk;
    final colour = !enabled
        ? t.textDisabled
        : _hover
            ? t.textPrimary
            : t.textSecondary;
    final ink = filled ? t.surface0 : t.accent;
    final Color? fill = !filled && !active && _hover ? t.surface4 : null;
    final disc = _lanternPill - 2 * t.tokens.pillInset;
    final decoration = desk
        ? BoxDecoration(color: fill)
        : BoxDecoration(
            color: fill,
            borderRadius: BorderRadius.circular(t.tokens.actionRadius),
          );
    final height = !widget.rail
        ? _toolButtonHeight
        : desk
            ? toolRailWidth
            : 36.0;
    // Arming a tool is a key press, many times an hour, so the hover light
    // goes at once from the tool armed and the one let go.
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
                        color: !active && _hover ? t.surface4 : null,
                      ),
                    ),
                  ),
                _Inked(
                  slide: _StripScope.of(context).slide,
                  ink: ink,
                  child: Stack(
                    children: [
                      Center(
                        child: TweenAnimationBuilder<Color?>(
                          tween: ColorTween(end: colour),
                          duration: step.duration,
                          curve: step.curve,
                          builder: (context, tint, _) => lumitIcon(member.icon,
                              size: iconSize, color: tint ?? colour),
                        ),
                      ),
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
          for (final member in ToolMode.builtMembersOf(group))
            MenuRow(
              key: ValueKey<String>('tool-flyout-${member.name}'),
              selected: member == armed,
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

class _WorkspaceStrip extends StatelessWidget {
  const _WorkspaceStrip();

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = context.watch<LumitUiState>();
    final app = context.read<LumitState>();
    final active = ui.workspace.activePreset;
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

  final bool moved;

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
