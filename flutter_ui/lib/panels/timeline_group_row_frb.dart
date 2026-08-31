// The Timeline's **layer group** rows (K-702): the header row in the outline,
// and the combined bar the lane half draws beside it.
//
// A group is a labelled band over a run of layers, and nothing more — it moves
// no layer, changes no blend, and the renderer has never heard of it. The row
// here is therefore deliberately thinner than a layer's: a twirl, a colour
// tick, a name, the four switches that broadcast to its members, and a bar
// spanning from the earliest member's in point to the latest one's out.
//
// **Precompose sits on this row's own menu.** The group is the light fold; the
// heavy one — packing the layers into a comp of their own, which does change
// the picture — is one click away on the same row, wired to the precompose
// road that already existed.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:uuid/uuid.dart';
import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/timeline_columns.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'timeline_extras_frb.dart';
import 'timeline_metrics_frb.dart';
import 'timeline_outline_frb.dart';

/// Fold the given layers into a new group, and answer whether it happened.
///
/// **One implementation, three callers** — Ctrl+G, the Layer menu's row and
/// anything that grows one later — which is the same rule Pre-compose already
/// follows. The refusal is the engine's: a selection that is not an unbroken
/// run of the stack, or one already in a group, comes back false and the stack
/// is left exactly as it was rather than rearranged to make the group possible.
bool groupSelectedLayers({
  required CompositionReference comp,
  required List<UuidValue> layerIds,
  required String name,
}) {
  if (layerIds.isEmpty) return false;
  try {
    comp.groupLayers(layerIds: layerIds, name: name);
    return true;
  } catch (_) {
    return false;
  }
}

/// Take away every group the given layers touch, and answer whether any went.
///
/// Every group rather than one, so Ctrl+Shift+G on a band picked by its header
/// row does the obvious thing without asking which of the layers in hand was
/// meant. A plain forward: the engine resolves the touched bands and commits
/// them as **one** undo step (K-720) — one commit per band was one undo step
/// per band, which is not what one keypress did.
bool ungroupSelection({
  required CompositionReference comp,
  required Set<UuidValue> layerIds,
}) =>
    comp.ungroupSelection(layerIds: [...layerIds]);

/// A group as one **carrier row** of the table sees it: the group itself, and
/// whether its fold is shut.
///
/// Carried on the topmost member's [LayerRow] rather than standing as a row of
/// its own, which is the whole reason the rest of the Timeline needed no
/// changes: `rows` stays one entry per visible layer, so the block heights, the
/// stack-drag arithmetic and both halves' [LazyBlocks] windows keep the shapes
/// K-638/K-678 gate. The header is simply drawn above its carrier's own row,
/// inside the carrier's block.
///
/// When [folded] is true the carrier draws **only** the header: its own row,
/// its fold-out and its bar all stand down, and the members below it are gone
/// from the row list entirely — the same filter the shy switch already
/// performs, which is why folding needed no new machinery either.
@immutable
class GroupHeader {
  final BridgeLayerGroup group;
  final bool folded;
  const GroupHeader(this.group, this.folded);

  String get id => group.id.toString();

  @override
  bool operator ==(Object other) =>
      other is GroupHeader && other.group == group && other.folded == folded;

  @override
  int get hashCode => Object.hash(group, folded);
}

/// What the group header row can be asked to do. One object rather than eight
/// callbacks threaded through two widget trees, because the outline row, the
/// lane bar and the context menu all want the same set.
@immutable
class GroupActions {
  /// Twirl the fold open or shut — session state, like a layer's own twirl.
  final ValueChanged<String> onToggleFold;

  /// Choose every member of the group (what makes the stack drag and every
  /// selection-wide command reach the whole band).
  final ValueChanged<BridgeLayerGroup> onSelect;
  final void Function(BridgeLayerGroup group, String name) onRename;
  final void Function(BridgeLayerGroup group, int label) onLabel;

  /// One of the four switches, broadcast to every member as one undo step.
  final void Function(BridgeLayerGroup group, BridgeGroupSwitch which, bool on)
      onSwitch;
  final ValueChanged<BridgeLayerGroup> onUngroup;

  /// The heavy fold: pack the group's members into a composition of their own,
  /// through the precompose road that already existed.
  final ValueChanged<BridgeLayerGroup> onPrecompose;

  /// The combined bar was dragged by this many frames — every member moves.
  final void Function(BridgeLayerGroup group, int deltaFrames) onShift;

  const GroupActions({
    required this.onToggleFold,
    required this.onSelect,
    required this.onRename,
    required this.onLabel,
    required this.onSwitch,
    required this.onUngroup,
    required this.onPrecompose,
    required this.onShift,
  });
}

/// The header row in the outline: twirl · colour tick · name · member count,
/// with the four broadcast switches in the switches column.
///
/// It draws its cells into the same column groups a layer's row does, at the
/// same widths, so the header sits *in* the table rather than across it — a
/// full-width band would have cut the switch columns in half every few rows.
class GroupOutlineRow extends StatefulWidget {
  final GroupHeader header;
  final List<TimelineGroup> groupOrder;
  final Map<TimelineGroup, double> widths;
  final GroupActions actions;

  /// The id `Enter` has just asked to rename (K-243's road, reached for a
  /// group when its header was the last thing chosen) — the panel's one
  /// notifier, shared with the layer rows: a group's id and a layer's are
  /// both [UuidValue]s, and only the row the value names has anything to do.
  final ValueNotifier<UuidValue?> renameRequest;
  const GroupOutlineRow({
    super.key,
    required this.header,
    required this.groupOrder,
    required this.widths,
    required this.actions,
    required this.renameRequest,
  });

  @override
  State<GroupOutlineRow> createState() => _GroupOutlineRowState();
}

class _GroupOutlineRowState extends State<GroupOutlineRow> {
  /// Open while the name is being typed into, exactly as a layer row's is.
  TextEditingController? _rename;

  BridgeLayerGroup get _group => widget.header.group;

  @override
  void initState() {
    super.initState();
    widget.renameRequest.addListener(_maybeRename);
  }

  @override
  void dispose() {
    widget.renameRequest.removeListener(_maybeRename);
    _rename?.dispose();
    super.dispose();
  }

  /// `Enter` with this group's band chosen names this header: open the editor
  /// on it, exactly as a layer row answers the same notifier.
  void _maybeRename() {
    if (!mounted || _rename != null) return;
    if (widget.renameRequest.value != _group.id) return;
    setState(() => _rename = TextEditingController(text: _group.name));
  }

  /// Clear the request this header answered, so pressing `Enter` again opens
  /// the editor a second time rather than seeing no change.
  void _clearRequest() {
    if (widget.renameRequest.value == _group.id) {
      widget.renameRequest.value = null;
    }
  }

  void _commitRename() {
    if (!mounted || _rename == null) return;
    final text = _rename?.text.trim() ?? '';
    setState(() {
      _rename?.dispose();
      _rename = null;
    });
    _clearRequest();
    if (text.isEmpty || text == _group.name) return;
    widget.actions.onRename(_group, text);
  }

  void _cancelRename() {
    if (!mounted || _rename == null) return;
    setState(() {
      _rename?.dispose();
      _rename = null;
    });
    _clearRequest();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      key: ValueKey<String>('tl-group-row-${widget.header.id}'),
      behavior: HitTestBehavior.opaque,
      // Choosing the header chooses the band: every command that runs over the
      // selection then runs over the whole group, which is what makes the
      // stack drag carry the members together without a second drag road.
      onTap: () => widget.actions.onSelect(_group),
      onSecondaryTapDown: (d) => _menu(context, d.globalPosition),
      child: Container(
        height: t.density.laneRow,
        decoration: BoxDecoration(
          // A shade above the panel's ground, so a band of rows reads as one
          // region without a border round it. `surface_2` is the raised value
          // the mockup gives a grouping strip.
          color: t.surface2,
        ),
        padding: const EdgeInsets.symmetric(horizontal: 8),
        child: Row(
          children: [
            for (var i = 0; i < widget.groupOrder.length; i++) ...[
              if (i > 0) rowSeam,
              SizedBox(
                width: widget.widths[widget.groupOrder[i]],
                child: switch (widget.groupOrder[i]) {
                  TimelineGroup.identity => _identity(t),
                  TimelineGroup.switches =>
                    _switches(t, widget.widths[TimelineGroup.switches] ?? 0),
                  // Every other column belongs to a layer, not to a band over
                  // one: a group has no blend mode, no matte and no parent,
                  // and a control that does nothing is worse than none.
                  _ => const SizedBox.shrink(),
                },
              ),
            ],
          ],
        ),
      ),
    );
  }

  Widget _identity(LumitTheme t) {
    final folded = widget.header.folded;
    return Row(
      children: [
        LumitTooltip(
          message: folded ? l10n.tipUnfoldGroup : l10n.tipFoldGroup,
          child: GestureDetector(
            key: ValueKey<String>('tl-group-twirl-${widget.header.id}'),
            behavior: HitTestBehavior.opaque,
            onTap: () => widget.actions.onToggleFold(widget.header.id),
            child: SizedBox(
              width: 16,
              height: t.density.laneRow,
              child: Center(
                child: glyph.LumitIcon(
                  folded ? LumitIcons.expand : LumitIcons.collapse,
                  size: iconSize,
                  colour: t.textPrimary,
                ),
              ),
            ),
          ),
        ),
        const SizedBox(width: identityGap),
        // Where a layer row puts its number, a group puts nothing: it has no
        // place in the stack of its own, and a blank keeps the names below it
        // in one column.
        const SizedBox(width: numberCellWidth),
        const SizedBox(width: identityGap),
        LumitTooltip(
          message: l10n.tipGroupColour,
          child: _tick(t),
        ),
        Expanded(child: _name(t)),
        // How many layers the fold holds — the one number worth showing on a
        // shut group, because it is the thing the fold hid.
        Padding(
          padding: const EdgeInsets.only(left: 6),
          child: Text('${_group.members.length}',
              style: t.mono.copyWith(fontSize: 10, color: t.textMuted)),
        ),
      ],
    );
  }

  Widget _name(LumitTheme t) {
    final editor = _rename;
    if (editor != null) {
      return HouseTextField(
        key: ValueKey<String>('tl-group-rename-${widget.header.id}'),
        controller: editor,
        autofocus: true,
        onSubmitted: (_) => _commitRename(),
        onTapOutside: _commitRename,
        onCancelled: _cancelRename,
      );
    }
    return GestureDetector(
      key: ValueKey<String>('tl-group-name-${widget.header.id}'),
      behavior: HitTestBehavior.opaque,
      onDoubleTap: () =>
          setState(() => _rename = TextEditingController(text: _group.name)),
      child: SizedBox(
        height: t.density.laneRow,
        child: Align(
          alignment: Alignment.centerLeft,
          // A group's name is a heading over the rows under it, so it carries
          // the kicker weight the outline gives its other headings rather than
          // a layer's body.
          child: Text(_group.name,
              style: t.bodyPrimary, overflow: TextOverflow.ellipsis),
        ),
      ),
    );
  }

  /// The colour tick: the same eight-colour picker a layer's label dot opens,
  /// drawn as a bar rather than a dot so a coloured group and a coloured layer
  /// inside it do not read as the same mark twice.
  Widget _tick(LumitTheme t) {
    return GestureDetector(
      key: ValueKey<String>('tl-group-label-${widget.header.id}'),
      behavior: HitTestBehavior.opaque,
      onTapDown: (d) async {
        final picked = await showLabelPicker(context, d.globalPosition,
            keyPrefix: 'tl-group-label');
        if (picked == null) return;
        widget.actions.onLabel(_group, picked);
      },
      child: SizedBox(
        width: 16,
        height: t.density.laneRow,
        child: Center(
          child: Container(
            width: 3,
            height: 11,
            decoration: BoxDecoration(
              color: t.labelColour(_group.label),
              borderRadius: BorderRadius.circular(1.5),
            ),
          ),
        ),
      ),
    );
  }

  /// The four broadcast switches, in the same cells and the same order a
  /// layer's row draws them. **A face reads on only when every member is on**
  /// (the engine answers that), so one press makes the whole group agree
  /// rather than flipping each member and leaving a mixed group mixed.
  Widget _switches(LumitTheme t, double width) {
    final shown = switchCellsFor(width);
    Widget cell(SwitchCell which) {
      final (on, sw, icon, tip) = switch (which) {
        SwitchCell.visible => (
            _group.visible,
            BridgeGroupSwitch.visible,
            LumitIcons.visible,
            l10n.tipGroupVisible
          ),
        SwitchCell.audible => (
            _group.audible,
            BridgeGroupSwitch.audible,
            LumitIcons.audio,
            l10n.tipGroupAudible
          ),
        SwitchCell.solo => (
            _group.solo,
            BridgeGroupSwitch.solo,
            LumitIcons.solo,
            l10n.tipGroupSolo
          ),
        SwitchCell.locked => (
            _group.locked,
            BridgeGroupSwitch.locked,
            _group.locked ? LumitIcons.lock : LumitIcons.unlocked,
            l10n.tipGroupLocked
          ),
        // Shy and the grid mark are per-layer filters, not things a band over
        // rows has an opinion about.
        _ => (false, null, null, null),
      };
      if (sw == null || !shown.contains(which)) {
        return SizedBox(width: switchCellWidth, height: t.density.laneRow);
      }
      return LumitTooltip(
        message: tip!,
        child: GestureDetector(
          key: ValueKey<String>('tl-group-${which.name}-${widget.header.id}'),
          behavior: HitTestBehavior.opaque,
          onTap: () => widget.actions.onSwitch(_group, sw, !on),
          child: SizedBox(
            width: switchCellWidth,
            height: t.density.laneRow,
            // The same whole-pixel inset a layer's switch cell uses (§6.20):
            // a 16px glyph centred in a 23px row lands on a half pixel, and
            // the icons carry a half-pixel nudge of their own.
            child: Align(
              alignment: Alignment.topLeft,
              child: Padding(
                padding: EdgeInsets.only(
                  left: wholePixelInset(switchCellWidth, iconSize),
                  top: wholePixelInset(t.density.laneRow, iconSize),
                ),
                child: glyph.LumitIcon(
                  icon!,
                  size: iconSize,
                  colour: on ? t.textPrimary : t.textMuted,
                ),
              ),
            ),
          ),
        ),
      );
    }

    return Row(
      children: [for (final which in SwitchCell.values) cell(which)],
    );
  }

  Future<void> _menu(BuildContext context, Offset position) async {
    final picked = await showMenuAt<String>(
      context: context,
      position: position,
      width: 200,
      rows: (close) => [
        MenuRow(
            key: const ValueKey('tl-group-menu-rename'),
            onPressed: () => close('rename'),
            child: Text(l10n.menuRenameGroup)),
        MenuRow(
            key: const ValueKey('tl-group-menu-ungroup'),
            onPressed: () => close('ungroup'),
            child: Text(l10n.menuUngroup)),
        // The heavy fold, one click from the light one (K-702). A group is
        // organisation; this is the same set of layers packed into a comp of
        // their own, which is what "collapse it all into a single layer"
        // actually means — and it is the existing precompose road, not a
        // second one.
        MenuRow(
            key: const ValueKey('tl-group-menu-precompose'),
            onPressed: () => close('precompose'),
            child: Text(l10n.menuPrecomposeGroup)),
      ],
    );
    if (!mounted) return;
    switch (picked) {
      case 'rename':
        setState(() => _rename = TextEditingController(text: _group.name));
      case 'ungroup':
        widget.actions.onUngroup(_group);
      case 'precompose':
        widget.actions.onPrecompose(_group);
    }
  }
}

/// The lane half of the header: the **combined bar**, spanning from the
/// earliest member's in point to the latest one's out.
///
/// Dragging it slides every member together, as one undo step
/// ([GroupActions.onShift]). Only a move — a group has no ends of its own to
/// trim, because trimming a band would have to decide which member's edge it
/// meant, and there is no honest answer to that.
class GroupBar extends StatefulWidget {
  final GroupHeader header;
  final GroupActions actions;

  /// Comp frames to pixels, and back — the axis the rest of the lane half
  /// draws against, so the bar lands on the same grid its members' bars do.
  final double Function(num frame) xOfFrame;
  final int Function(double dx) framesOfDx;
  const GroupBar({
    super.key,
    required this.header,
    required this.actions,
    required this.xOfFrame,
    required this.framesOfDx,
  });

  @override
  State<GroupBar> createState() => _GroupBarState();
}

class _GroupBarState extends State<GroupBar> {
  /// The drag's travel in frames while the hand is on it, staged rather than
  /// committed — the rule every other drag in this panel follows, so sixty
  /// pointer moves a second are sixty repaints of one bar rather than sixty
  /// document edits.
  int _delta = 0;
  double _dx = 0;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final g = widget.header.group;
    final left = widget.xOfFrame(g.inFrame + _delta);
    final right = widget.xOfFrame(g.outFrame + _delta);
    return SizedBox(
      height: t.density.laneRow,
      child: Stack(
        children: [
          Positioned(
            left: left,
            width: (right - left).clamp(1.0, double.infinity),
            top: 3,
            bottom: 3,
            child: GestureDetector(
              key: ValueKey<String>('tl-group-bar-${widget.header.id}'),
              behavior: HitTestBehavior.opaque,
              onHorizontalDragUpdate: (d) {
                _dx += d.delta.dx;
                setState(() => _delta = widget.framesOfDx(_dx));
              },
              onHorizontalDragEnd: (_) {
                final moved = _delta;
                setState(() {
                  _delta = 0;
                  _dx = 0;
                });
                if (moved != 0) widget.actions.onShift(g, moved);
              },
              onHorizontalDragCancel: () => setState(() {
                _delta = 0;
                _dx = 0;
              }),
              child: Container(
                decoration: BoxDecoration(
                  // The group's own colour, at the weight a bar carries — the
                  // band and its bar are one object, and giving the bar a
                  // different colour from the tick beside it would say
                  // otherwise.
                  color: t.labelColour(g.label).withValues(alpha: 0.35),
                  border: Border.all(color: t.labelColour(g.label)),
                  borderRadius: BorderRadius.circular(t.tokens.controlRadius),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}
