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
// **The magnet is back**. It sat here once, was taken off when nothing
// in the application read it — a toggle that changes nothing is worse
// than a missing one — and returns now that the Viewer's layer drags reach for
// the guides and the grid with it. Docs/07 §4.5 puts the switch that governs a
// gesture on the toolbar; the Viewer's own guides menu shows the same switch
// under its own name.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart' show BridgeBrushShape;
import 'package:provider/provider.dart';

import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../main.dart';
import '../panels/viewer_paint.dart' show brushShapeLabel;
import '../state/dock.dart';
import '../state/tools.dart';
import '../theme/theme.dart';
import '../widgets/colour_picker.dart';
import '../widgets/controls.dart';

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

class LumitToolBarFrb extends StatelessWidget {
  const LumitToolBarFrb({super.key});

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = context.watch<LumitUiState>();
    return Container(
      height: toolBarHeight,
      decoration: BoxDecoration(
        color: t.surface2,
        border: Border(bottom: BorderSide(color: t.hairline)),
      ),
      child: ListenableBuilder(
        listenable: ui.tools,
        builder: (context, _) => Row(
          children: [
            const SizedBox(width: 4),
            // The tools and their options take the whole left-hand end, so the
            // workspace strip is held against the *right* edge where docs/07
            // §1.4 puts it. Expanded rather than letting the two scroll views
            // size themselves: a loose Flexible only takes the width it needs,
            // which left the workspace buttons sitting immediately beside the
            // last tool with the free space stranded past them.
            Expanded(
              child: Row(
                children: [
                  // Scrolls rather than overflowing: a narrow window has less
                  // width than thirteen tools want, and an overflow stripe is
                  // not a design. Flexible so the options beside it get their
                  // share of the room instead of being squeezed to nothing.
                  Flexible(
                    child: SingleChildScrollView(
                      scrollDirection: Axis.horizontal,
                      child: Row(
                        children: [
                          for (final group in toolBarOrder)
                            _ToolButton(group: group, tools: ui.tools),
                        ],
                      ),
                    ),
                  ),
                  // The armed tool's own options, when it has any:
                  // After Effects puts them here, and the strip is empty for
                  // the tools that draw nothing.
                  if (toolOptionsFor(ui.tools.tool) != ToolOptions.none) ...[
                    const _ToolBarDivider(),
                    Flexible(
                      child: SingleChildScrollView(
                        scrollDirection: Axis.horizontal,
                        child: _ToolOptions(
                          tools: ui.tools,
                          shows: toolOptionsFor(ui.tools.tool),
                        ),
                      ),
                    ),
                  ],
                ],
              ),
            ),
            const _ToolBarDivider(),
            _SnapButton(tools: ui.tools),
            const _ToolBarDivider(),
            const _WorkspaceStrip(),
            const SizedBox(width: 6),
          ],
        ),
      ),
    );
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
      child: HouseButton(
        key: const ValueKey('tool-snapping'),
        small: true,
        frameless: !on,
        padding: const EdgeInsets.symmetric(horizontal: 4),
        onPressed: () => tools.snapping = !on,
        child: lumitIcon(LumitIcon.magnet,
            size: iconSize, color: on ? t.textPrimary : t.textMuted),
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
}

/// The options [tool] shows on the toolbar.
ToolOptions toolOptionsFor(ToolMode tool) => switch (tool.group) {
      ToolGroup.type => ToolOptions.type,
      ToolGroup.paint => ToolOptions.paint,
      ToolGroup.shape || ToolGroup.pen => ToolOptions.shape,
      ToolGroup.puppet => ToolOptions.puppet,
      ToolGroup.roto => ToolOptions.roto,
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
          onPicked: (colour) => tools.fill = colour,
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
            padding: const EdgeInsets.only(right: 6),
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
            padding: const EdgeInsets.only(right: 6),
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
            onPicked: (colour) => tools.stroke = colour,
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
      padding: const EdgeInsets.only(right: 6),
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
class _Swatch extends StatelessWidget {
  final String label;
  final ToolColour colour;
  final ValueChanged<ToolColour>? onPicked;

  const _Swatch({
    required this.label,
    required this.colour,
    required this.onPicked,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final swatch = Container(
      width: 20,
      height: 20,
      decoration: BoxDecoration(
        color: PickedColour(colour.r, colour.g, colour.b).clipped,
        border: Border.all(color: t.hairlineStrong),
        borderRadius: BorderRadius.circular(3),
      ),
    );
    final row = Row(
      children: [
        Text(label, style: t.small.copyWith(color: t.textSecondary)),
        const SizedBox(width: 5),
        swatch,
      ],
    );
    if (onPicked == null) return row;
    return LumitTooltip(
      message: l10n.tipSwatchColour(label),
      child: Builder(
        builder: (context) => MouseRegion(
          cursor: SystemMouseCursors.click,
          child: GestureDetector(
            onTapUp: (details) => showColourPicker(
              context: context,
              position: details.globalPosition,
              initial: PickedColour(colour.r, colour.g, colour.b),
              scale: ColourScale.bytes,
              onCommit: (picked) => onPicked!(
                ToolColour(picked.r, picked.g, picked.b),
              ),
            ),
            child: row,
          ),
        ),
      ),
    );
  }
}

class _ToolBarDivider extends StatelessWidget {
  const _ToolBarDivider();

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      width: 1,
      height: 20,
      margin: const EdgeInsets.symmetric(horizontal: 6),
      color: t.hairline,
    );
  }
}

/// One group's button: the member it stands for, armed by a click, with the
/// rest of the group behind a press-and-hold or a right-click.
class _ToolButton extends StatefulWidget {
  final ToolGroup group;
  final ToolsState tools;

  const _ToolButton({required this.group, required this.tools});

  @override
  State<_ToolButton> createState() => _ToolButtonState();
}

class _ToolButtonState extends State<_ToolButton> {
  bool _hover = false;

  @override
  Widget build(BuildContext context) {
    final scope = ThemeScope.of(context);
    final t = scope.theme;
    final member = widget.tools.memberOf(widget.group);
    final active = widget.tools.tool.group == widget.group;
    final members = ToolMode.membersOf(widget.group);
    // A group nothing in which is built is on the strip but cannot be pressed:
    // the tool set is the specification, and a button that visibly
    // cannot be pressed says "coming" where a missing one says nothing.
    final enabled = ToolMode.builtMembersOf(widget.group).isNotEmpty;

    // 15-DESIGN §5's icon states, exactly: secondary at rest, primary on hover,
    // accent when this is the tool in your hand — and muted for a group that
    // cannot be armed at all.
    final colour = !enabled
        ? t.textDisabled
        : active
            ? t.accent
            : _hover
                ? t.textPrimary
                : t.textSecondary;

    return LumitTooltip(
      message: _tooltip(context, member, members.length > 1),
      child: MouseRegion(
        cursor: enabled ? SystemMouseCursors.click : SystemMouseCursors.basic,
        onEnter: (_) => setState(() => _hover = enabled),
        onExit: (_) => setState(() => _hover = false),
        child: GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTap: enabled ? () => widget.tools.select(member) : null,
          // Both routes to the hidden tools, because both are muscle memory:
          // After Effects opens the flyout on a press-and-hold, and every other
          // toolbar on this machine opens a menu on the right button.
          onLongPress:
              enabled && members.length > 1 ? () => _openFlyout(context) : null,
          onSecondaryTapUp: enabled && members.length > 1
              ? (_) => _openFlyout(context)
              : null,
          child: AnimatedContainer(
            key: ValueKey<String>('tool-${widget.group.name}'),
            duration: animationDuration(scope.animationLevel),
            width: _toolButtonWidth,
            height: _toolButtonHeight,
            decoration: BoxDecoration(
              color: active
                  ? t.accent.withValues(alpha: 0.16)
                  : _hover
                      ? t.surface4
                      : null,
              borderRadius: BorderRadius.circular(t.tokens.controlRadius),
            ),
            child: Stack(
              children: [
                Center(
                    child:
                        lumitIcon(member.icon, size: iconSize, color: colour)),
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

  /// The tooltip: the tool's name, its shortcut as this machine spells it, and
  /// — for a tool whose behaviour is not built — the plain fact that arming it
  /// changes nothing yet. Saying so is cheaper than a user discovering it by
  /// dragging and getting silence.
  String _tooltip(BuildContext context, ToolMode member, bool hasHidden) {
    final chord =
        context.read<LumitUiState>().keymap.chordFor(_actionFor(widget.group));
    final parts = <String>[
      chord == null ? member.label : '${member.label} ($chord)',
      if (hasHidden) l10n.tipMoreInGroup,
      if (!member.ready) l10n.tipNotBuiltYet,
    ];
    return parts.join(' · ');
  }

  void _openFlyout(BuildContext context) {
    final box = context.findRenderObject();
    if (box is! RenderBox) return;
    final origin = box.localToGlobal(Offset(0, box.size.height));
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
          for (final member in ToolMode.membersOf(group))
            MenuRow(
              key: ValueKey<String>('tool-flyout-${member.name}'),
              selected: member == armed,
              // A member that is not built is listed and does nothing when
              // clicked — the same rule the buttons follow.
              onPressed: member.ready ? () => onPick(member) : () {},
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

/// The workspace switcher docs/07 §1.4 requires in the window chrome: the
/// shipped presets as mono-caps kickers, the current one ticked in the accent.
class _WorkspaceStrip extends StatelessWidget {
  const _WorkspaceStrip();

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final ui = context.watch<LumitUiState>();
    final active = ui.workspace.activePreset;
    // Round's filled pill (§12.1): five names, one in force, which is
    // exactly the segmented option the cue is about, and the fill replaces the
    // tick. Sharp keeps the underline: passing `active` there would give the
    // word a fill it has never had.
    final round = t.shape == ThemeShape.round;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        for (final preset in WorkspacePreset.values)
          _StripEntry(
            key: ValueKey<String>('workspace-${preset.name}'),
            label: preset.title,
            active: preset == active,
            round: round,
            onPressed: () => ui.workspace.applyWorkspacePreset(preset),
          ),
        // The user's own, after the presets and in the same order the chords
        // count (docs/07 §1.4). Drawn by exactly the same rules — a workspace
        // somebody saved is a workspace, not a lesser kind of one.
        for (final saved in ui.workspace.userWorkspaces)
          _StripEntry(
            key: ValueKey<String>('workspace-user-${saved.name}'),
            label: saved.name,
            active: saved.name == ui.workspace.activeUserWorkspace,
            round: round,
            onPressed: () => ui.workspace.applyUserWorkspace(saved.name),
          ),
      ],
    );
  }
}

/// One name on the workspace strip — a shipped preset or one of the user's
/// own, drawn identically because they are the same kind of thing.
///
/// Mono-caps kickers with an **accent tick under the one in force** (docs/15
/// §12A.1, §3.1 — the workspace tabs are what "the active tab tick" means).
/// The word itself stays grey: the tick is the state, so the strip reads as
/// names with one underlined rather than as one coloured word. Under Round the
/// filled pill (§12.1) carries the state instead, and there is no tick
/// to draw under a fill.
///
/// The padding is what fits the name into the strip, and the strip is 14px
/// shorter than it was: at 12 above and below, 24px of padding in a
/// 30px band left the words with five and they were squeezed out of sight,
/// leaving a button that could be pressed and not read.
class _StripEntry extends StatelessWidget {
  final String label;
  final bool active;
  final bool round;
  final VoidCallback onPressed;

  const _StripEntry({
    super.key,
    required this.label,
    required this.active,
    required this.round,
    required this.onPressed,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return LumitTooltip(
      message: l10n.tipPanelLayout,
      child: HouseButton(
        frameless: true,
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
        active: round && active,
        onPressed: onPressed,
        child: Container(
          padding: const EdgeInsets.only(bottom: 2),
          decoration: active && !round
              ? BoxDecoration(
                  border: Border(bottom: BorderSide(color: t.accent)))
              : null,
          child: Text(
            label.toUpperCase(),
            style: active
                ? (round ? t.kicker.copyWith(color: t.surface0) : t.kickerOn)
                : t.kicker,
          ),
        ),
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
              tool.group == ToolGroup.pen ||
              tool.group == ToolGroup.paint ||
              tool.group == ToolGroup.roto ||
              tool.group == ToolGroup.puppet ||
              tool.group == ToolGroup.camera
          ? SystemMouseCursors.precise
          : SystemMouseCursors.basic,
    };
