// One row of the Timeline's outline: the number, the label dot, the name, the
// switches, the blend mode and the parent picker.
//
// Split out of timeline_panel_frb.dart. It is one class doing one job — a row
// — and stayed whole.

import 'package:flutter/gestures.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/l10n/engine_labels.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';
import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../shell/menu_bar_frb.dart' show duplicateLayersFrb;
import '../shell/stretch_dialog_frb.dart';
import '../state/timeline_columns.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'timeline_extras_frb.dart';
import 'sequence_view_frb.dart';
import 'timeline_timings.dart';
import 'timeline_metrics_frb.dart';
import 'timeline_outline_frb.dart';
import 'transform_rows_frb.dart' show hasThreeDSwitch, hasVisibilitySwitch;

/// The blend-mode names, fetched once per session: the list is static for the
/// life of the process, and every outline row was re-fetching it per rebuild.
List<String>? _blendModes;

/// The label-colour dot beside a layer's name — the mockup's own 6px bullet,
/// drawn round whatever the shape is: this is a colour swatch, not a control,
/// and Sharp's square corners have nothing to say about a bullet.
const double _labelDotSize = 6;

/// A switch being painted down its column: which switch, the layer the drag
/// started on, and the state that layer took, which every row the pointer
/// crosses takes too.
typedef _SwitchPaint = ({String cell, UuidValue from, bool to});

/// The bar at a row's leading edge that says someone else in a shared project
/// has the layer selected.
const double _othersBarWidth = 2;

/// The inline rename a row turns into while it is being named: `Enter`
/// commits, Escape throws the edit away, and a click anywhere else commits too
/// (the field loses the row).
///
/// One field for two tables. A layer's row here and a track's row in the Audio
/// timeline are named the same way, and a second copy of this would be a
/// second set of answers about what Escape means.
class RowRenameField extends StatelessWidget {
  final TextEditingController controller;
  final VoidCallback onCommit;
  final VoidCallback onCancel;

  const RowRenameField({
    super.key,
    required this.controller,
    required this.onCommit,
    required this.onCancel,
  });

  @override
  Widget build(BuildContext context) => HouseTextField(
        controller: controller,
        autofocus: true,
        onSubmitted: (_) => onCommit(),
        onTapOutside: onCommit,
        onCancelled: onCancel,
      );
}

class OutlineRow extends StatefulWidget {
  final CompositionReference comp;
  final BridgeLayerEntry entry;

  /// Open or close this layer's sequence view — what a double-click
  /// on a Sequence layer means, where on other kinds it opens the source.
  final VoidCallback? onOpenSequence;

  /// Every layer in the comp, for the parent picker's menu — from the same
  /// read model, so offering them costs nothing.
  final List<BridgeLayerEntry> layers;

  /// The column groups in their current order, and their current widths
  /// (docs/07 §4.2).
  final List<TimelineGroup> groupOrder;
  final Map<TimelineGroup, double> widths;

  /// Whether the matte column carries its mode toggles' room — the
  /// panel's answer for the whole comp, not this row's: a row with no matte
  /// still leaves the slot when a row above it has one.
  final bool matteToggles;
  final int index;
  final int count;
  final bool selected;

  /// A sub-item of this layer was last touched — drawn a shade dimmer than
  /// selection, so the two states read apart at a glance.
  final bool highlighted;

  /// The colour of each other person in a shared project who has this layer
  /// selected, drawn as a bar at the row's leading edge.
  final List<int> others;
  final bool open;

  /// What this layer can do, so the switches column offers only that:
  /// no audible switch where there is no sound, no visibility switch where
  /// there is no picture. Passed down from the panel — probing for either
  /// answer must never happen in a row's build.
  final bool hasAudio;
  final bool hasPicture;
  final VoidCallback onToggleOpen;
  final VoidCallback onSelect;
  final VoidCallback onChanged;

  /// The panel's drag state: this row is where the gesture is made — the name
  /// is the stack handle — and setting it here is what lets the lanes beside
  /// the outline move with it.
  final LayerDragState layerDrag;

  /// The layer the panel has just been asked to rename (`Enter`), or
  /// null. A notifier rather than a rebuild because only the one row it names
  /// has anything to do about it.
  final ValueNotifier<UuidValue?> renameRequest;

  /// Every block's height, as the stack stood when the panel last built —
  /// what a drag's travel is measured against, so the answer does not depend
  /// on rows the drag is itself moving.
  final List<double> blockHeights;

  const OutlineRow({
    super.key,
    required this.comp,
    required this.entry,
    this.onOpenSequence,
    required this.layers,
    required this.groupOrder,
    required this.widths,
    required this.matteToggles,
    required this.index,
    required this.count,
    required this.selected,
    required this.highlighted,
    this.others = const [],
    required this.open,
    this.hasAudio = false,
    this.hasPicture = true,
    required this.onToggleOpen,
    required this.onSelect,
    required this.onChanged,
    required this.layerDrag,
    required this.renameRequest,
    required this.blockHeights,
  });

  @override
  State<OutlineRow> createState() => _OutlineRowState();
}

/// The rows on screen, by layer, so a layer's bar can open its row's menu.
final Map<UuidValue, _OutlineRowState> _rowsOnScreen = {};

/// The menu a right-click on [layer]'s row opens, at [position]. Nothing
/// when that row is not on screen.
void showLayerRowMenu(UuidValue layer, Offset position) {
  final row = _rowsOnScreen[layer];
  if (row != null) row._showRowMenu(row.context, position);
}

class _OutlineRowState extends State<OutlineRow> {
  /// The inline rename, entered with `Enter` on the selected layer.
  TextEditingController? _rename;

  /// How far this row has been dragged since the lift, in pixels down.
  ///
  /// Accumulated from the gesture's own deltas rather than read back off the
  /// widget's position, because the widget is being slid by the drag: its
  /// position is an output of this number, so reading it back would be the
  /// loop the travel measure exists to break.
  double _dragTravel = 0;

  /// Put the layer where the drag says, and let the rows go.
  ///
  /// A drop that lands where it started is not a reorder — it is the user
  /// changing their mind, and it must cost nothing. Committing it anyway
  /// wrote an undo step for a stack that had not moved.
  void _commitDrag() {
    // Letting go first: the blocks take their landing from the drag state,
    // and the reorder below rebuilds the rows in the same frame, so the row
    // in hand settles into a stack that is already in its new order.
    final drag = widget.layerDrag.release(widget.blockHeights);
    if (drag == null || drag.from == drag.to) return;
    widget.layers[drag.from].layer
        .reorder(newIndex: BigInt.from(_stackIndex(drag.to)));
    widget.onChanged();
  }

  /// Where the row at [at] on screen stands in the whole comp - what the
  /// engine counts. The rows on screen may be a filtered list: the shy
  /// filter, the search box, the Sound mix fold. Handed a
  /// slot in that list, a reorder landed a layer somewhere else in the stack
  /// whenever anything was hidden; taking the place of the layer that is
  /// *in* the slot is what the drop means whichever rows are showing.
  int _stackIndex(int at) {
    if (at < 0 || at >= widget.layers.length) return at;
    final id = widget.layers[at].layer.internallayerId;
    final all =
        Provider.of<LumitUiState>(context, listen: false).model.heldLayers;
    final i = all.indexWhere((e) => e.layer.internallayerId == id);
    return i < 0 ? at : i;
  }

  LayerReference get layer => widget.entry.layer;
  int get index => widget.index;
  int get count => widget.count;

  /// What a command invoked on this row acts on: **the whole selection
  /// when this row is part of it, and this row alone when it is not**.
  ///
  /// The same rule the Project panel's `_targets` states — a right-click on an
  /// unpicked row is about that row, and everything else is about what is
  /// picked. Returned in stack order, from the panel's own layer list, so a
  /// reorder can count on the order it reads.
  ///
  /// Read from the shell rather than passed down, and only ever from a
  /// handler: a row's build must not ask what is selected beyond the `selected`
  /// flag it is already given.
  List<BridgeLayerEntry> _menuTargets() {
    final picked =
        Provider.of<LumitUiState>(context, listen: false).selectedLayerIds;
    if (!picked.contains(layer.internallayerId)) return [widget.entry];
    final targets = [
      for (final e in widget.layers)
        if (picked.contains(e.layer.internallayerId)) e,
    ];
    return targets.isEmpty ? [widget.entry] : targets;
  }

  @override
  void initState() {
    super.initState();
    widget.renameRequest.addListener(_maybeRename);
    lumitPopupUp.addListener(_menuGone);
    _rowsOnScreen[layer.internallayerId] = this;
  }

  @override
  void dispose() {
    // Only its own entry: a row rebuilt elsewhere has already taken the slot.
    if (_rowsOnScreen[layer.internallayerId] == this) {
      _rowsOnScreen.remove(layer.internallayerId);
    }
    widget.renameRequest.removeListener(_maybeRename);
    lumitPopupUp.removeListener(_menuGone);
    _rename?.dispose();
    super.dispose();
  }

  /// `Enter` on the selected layer names this row: open the editor on it.
  /// A locked layer keeps its name, the same as it did when a double-click was
  /// what opened the editor — lock means no edits.
  void _maybeRename() {
    if (!mounted || _rename != null) return;
    if (widget.renameRequest.value != layer.internallayerId) return;
    if (widget.entry.info.switches.locked) return;
    setState(
        () => _rename = TextEditingController(text: widget.entry.info.name));
  }

  /// Escape: shut the editor and rename nothing. Shares the closing
  /// half of [_commitRename] — the write is the only difference between them.
  void _cancelRename() {
    if (!mounted || _rename == null) return;
    setState(() {
      _rename?.dispose();
      _rename = null;
    });
    if (widget.renameRequest.value == layer.internallayerId) {
      widget.renameRequest.value = null;
    }
  }

  void _commitRename() {
    // Both ways out of the editor can land here for one edit — submitting and
    // then losing the pointer — and the row can be gone by the time the second
    // arrives. Either way there is nothing left to commit.
    if (!mounted || _rename == null) return;
    final text = _rename?.text.trim() ?? '';
    setState(() {
      _rename?.dispose();
      _rename = null;
    });
    // Clear the request this row answered, so pressing Enter again on the same
    // layer opens the editor a second time rather than seeing no change.
    if (widget.renameRequest.value == layer.internallayerId) {
      widget.renameRequest.value = null;
    }
    if (text.isEmpty || text == widget.entry.info.name) return;
    layer.rename(name: text);
    widget.onChanged();
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // ZERO bridge calls: everything this row draws is in the read model.
    final info = widget.entry.info;

    // The row knows when the pointer is over it and when one of its controls
    // has the keyboard: what rests quietly comes up then.
    final body = MouseRegion(
      opaque: false,
      onEnter: (_) => setState(() => _hover = true),
      onExit: (e) {
        if (!mounted) return;
        // A menu opened from this row takes the pointer without it moving:
        // the pointer is still inside the row and a menu is up. The row then
        // keeps its full form until the menu goes.
        final box = context.findRenderObject();
        setState(() {
          _hover = false;
          _underMenu = lumitPopupOpen &&
              box is RenderBox &&
              box.size.contains(box.globalToLocal(e.position));
        });
      },
      child: Focus(
        canRequestFocus: false,
        skipTraversal: true,
        onFocusChange: (has) {
          if (mounted) setState(() => _focus = has);
        },
        child: _rowBody(context, t, info),
      ),
    );

    // Selection happens on the DOWN, for the whole row, outside the gesture
    // arena — the reason the name has always done it that way (see the note by
    // the name cell) applies to every other cell too, and the row's tap used to
    // do it a *second* time on the way up. Two calls per click is invisible for
    // a plain click and exactly wrong for a Ctrl+click, which toggled the layer
    // in and straight back out again.
    return Listener(
      onPointerDown: (event) {
        if (_claimed) {
          _claimed = false;
          return;
        }
        if (event.buttons == kPrimaryButton) widget.onSelect();
      },
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        // A tap that does nothing, so that nothing is what happens: the empty
        // ground behind these rows deselects on tap, and a row that
        // entered no tap into the arena let the ground win and throw away the
        // selection the pointer-down had just made.
        onTap: () {},
        onSecondaryTapDown: (d) => _showRowMenu(context, d.globalPosition),
        child: Container(
          // No drop line: the rows themselves move to where they would land,
          // so a line marking the same slot said it twice.
          child: body,
        ),
      ),
    );
  }

  /// Set by a control on its way down, so the row above it leaves the
  /// selection alone: pressing a layer's eye, or opening its properties, is
  /// not choosing the layer. The gesture arena used to settle this by itself,
  /// and cannot now that the row selects from a raw listener outside it.
  ///
  /// Cleared by the very next pointer-down the row sees, which is this same
  /// one — Flutter hands a pointer to the innermost target first, so the
  /// control always sets this before the row reads it.
  bool _claimed = false;

  /// Whether the pointer is over this row, and whether one of its controls
  /// has the keyboard. At rest, with neither, an off switch draws dim and a
  /// picker at its default draws as its word alone.
  bool _hover = false;
  bool _focus = false;

  /// Whether a menu opened from this row is up. Its barrier takes the pointer
  /// away, and a picker must not go quiet under its own open menu.
  bool _underMenu = false;
  bool get _resting => !_hover && !_focus && !_underMenu;

  /// The menu has gone. After the frame, so a pointer that is back over the
  /// row has been seen and the row does not rest for one frame in between.
  void _menuGone() {
    if (!_underMenu || lumitPopupUp.value) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (mounted) setState(() => _underMenu = false);
    });
  }

  /// Mark [child]'s clicks as the control's own, not the row's.
  Widget _ownClick(Widget child) =>
      Listener(onPointerDown: (_) => _claimed = true, child: child);

  Widget _rowBody(BuildContext context, LumitTheme t, BridgeLayerInfo info) {
    // The pitch outside, the drawn row inside: Lantern leaves a pixel of
    // ground at each edge of the row and rounds it, as the fold rows do.
    final body = Container(
        height: t.density.laneRow,
        padding: EdgeInsets.symmetric(vertical: laneRowGap(t)),
        child: Container(
            key: ValueKey<String>('tl-rowbody-${layer.internallayerId}'),
            decoration: BoxDecoration(
              // Selected is the brighter of the two states; a highlight (this
              // layer's fold-out was last touched) is the same surface at half
              // strength, so they read apart at a glance.
              //
              // In hand it draws neither: the lifted card under the row is
              // that fill already, and it runs on into the lanes.
              color: LayerLift.of(context)
                  ? null
                  : widget.selected
                      ? rowSelectionFill(t)
                      : widget.highlighted
                          ? rowSelectionFill(t).withValues(alpha: 0.45)
                          : null,
              // No seam of its own: the overlay draws the seams for the whole
              // outline, and a border here drew a *second* line a fraction of a
              // pixel from it, the overlay is phased by the scroll offset, which
              // a trackpad leaves fractional, so the two lines pulled apart as the
              // table scrolled and the outline's rows read a hair taller than the
              // lanes beside them.
              borderRadius: t.shape == ThemeShape.lantern
                  ? BorderRadius.circular(t.tokens.controlRadius)
                  : null,
            ),
            padding: const EdgeInsets.symmetric(horizontal: 8),
            child: Row(
              children: [
                // The cells come in the four column groups, in whatever order
                // the header's drag has put them and at whatever width its seams
                // have been dragged to (docs/07 §4.2).
                for (var i = 0; i < widget.groupOrder.length; i++) ...[
                  if (i > 0) rowSeam,
                  SizedBox(
                    width: widget.widths[widget.groupOrder[i]],
                    // Only the identity group is the layer itself, its name and
                    // its number are what you click to choose it. The other three
                    // are controls: hiding a layer, or picking its blend mode, is
                    // not choosing it, and those cells have never selected.
                    child: switch (widget.groupOrder[i]) {
                      TimelineGroup.identity =>
                        _identityCells(context, t, info),
                      TimelineGroup.switches => _ownClick(_switchCells(context,
                          t, info, widget.widths[TimelineGroup.switches] ?? 0)),
                      TimelineGroup.render => _ownClick(_renderCells(context,
                          info, widget.widths[TimelineGroup.render] ?? 0)),
                      TimelineGroup.compose => _ownClick(_composeCells(context,
                          t, info, widget.widths[TimelineGroup.compose] ?? 0)),
                      TimelineGroup.parent => _ownClick(_parentCell(
                          info, widget.widths[TimelineGroup.parent] ?? 0)),
                      // What this layer's own picture cost in the last measured
                      // frame (docs/13 §7.1). A readout, not a control: it neither
                      // selects the layer nor claims the click.
                      TimelineGroup.timings => TimingsCell(
                          layerId: layer.internallayerId.toString(),
                        ),
                    },
                  ),
                ],
              ],
            )));
    // What the others have selected: a bar at the row's leading edge, shared
    // out between their colours. Over the fill, so it still shows on a row
    // this person has selected too. The stack is there either way, so a mark
    // arriving does not rebuild the row from nothing under a rename.
    return Stack(
      // The row keeps the tight width it was always laid out at, which is
      // what makes it a layout boundary. Left loose, the edit row of
      // `rebuild_budget_test` fails.
      fit: StackFit.passthrough,
      children: [
        body,
        if (widget.others.isNotEmpty)
          Positioned(
            left: 0,
            top: laneRowGap(t),
            bottom: laneRowGap(t),
            width: _othersBarWidth,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                for (final colour in widget.others)
                  Expanded(child: ColoredBox(color: t.personColour(colour))),
              ],
            ),
          ),
      ],
    );
  }

  /// Group 1: visibility · audio · solo · lock · shy. The first two swap
  /// their glyph when off — a closed eye, a muted speaker — rather than only
  /// dimming, so the off state reads at a glance.
  ///
  /// **Only what the layer can do**. The eye is drawn for a layer with
  /// a picture, the speaker for a layer with sound — so an Audio layer has no
  /// eye, and a solid, a title, a shape or an image-only clip has no speaker.
  /// A control that does nothing when clicked is worse than no control: you
  /// have to click it to find out. Each keeps its cell's width either way, so
  /// the switches stay in their columns down the stack and the ones a row does
  /// have sit where the eye reads for them.
  ///
  /// **And only what the column has room for** (T4): dragged narrower it gives
  /// its cells up in [switchHideOrder] — the grid mark, then shy, then lock,
  /// then solo — and visibility and audio are never among them.
  Widget _switchCells(
      BuildContext context, LumitTheme t, BridgeLayerInfo info, double width) {
    final id = layer.internallayerId.toString();
    final switches = info.switches;
    final blank = SizedBox(width: switchCellWidth, height: t.density.laneRow);
    Widget cell(SwitchCell which) => switch (which) {
          SwitchCell.visible =>
            hasVisibilitySwitch(info.kind, hasPicture: widget.hasPicture)
                ? _switch(context, id, 'visible', null, switches.visible,
                    BridgeLayerSwitch.visible,
                    mark: LumitIcons.visible,
                    offMark: LumitIcons.hidden,
                    tip: switches.visible
                        ? l10n.switchVisible
                        : l10n.switchHidden)
                : blank,
          SwitchCell.audible => widget.hasAudio
              ? _switch(context, id, 'audible', null, switches.audible,
                  BridgeLayerSwitch.audible,
                  mark: LumitIcons.audio,
                  offMark: LumitIcons.muted,
                  tip: switches.audible ? l10n.switchAudible : l10n.switchMuted)
              : blank,
          // A ringed dot, dimmed until soloed — the set has one solo mark, so
          // this pair is told apart by strength rather than by shape.
          SwitchCell.solo => _switch(
              context, id, 'solo', null, switches.solo, BridgeLayerSwitch.solo,
              mark: LumitIcons.solo,
              offMark: LumitIcons.solo,
              tip: l10n.switchSoloAlone(
                  switches.solo ? l10n.switchSoloed : l10n.switchSolo)),
          SwitchCell.locked => _switch(context, id, 'locked', null,
              switches.locked, BridgeLayerSwitch.locked,
              mark: LumitIcons.lock,
              offMark: LumitIcons.unlocked,
              tip: switches.locked ? l10n.switchLocked : l10n.switchLock),
          SwitchCell.shy => _switch(context, id, 'shy', LumitIcon.shyHidden,
              switches.shy, BridgeLayerSwitch.shy,
              offIcon: LumitIcon.shy,
              tip: switches.shy ? l10n.switchShy : l10n.switchMarkShy),
          // Guide, the cell beside shy that docs/07 §4.2 names for it.
          // **Drawn on every row**, unlike the kind-gated cells in the Modes
          // column: any layer can be reference-only — a match photograph, a
          // grid, an animatic — so there is no kind the mark would do nothing
          // on. The two strengths are the column's own: lit `text_primary`
          // while the layer is a guide, resting at `text_muted` when it is not.
          SwitchCell.guide => _switch(context, id, 'guide', null,
              switches.guide, BridgeLayerSwitch.guide,
              mark: LumitIcons.guide,
              tip: switches.guide ? l10n.switchGuide : l10n.switchMarkGuide),
        };
    final shown = switchCellsFor(width);
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        for (final which in SwitchCell.values)
          if (shown.contains(which)) cell(which),
      ],
    );
  }

  /// Group 2: twirl · layer number · label dot · name (the mockup's own order;
  /// the dot and the number used to stand the other way round).
  Widget _identityCells(
      BuildContext context, LumitTheme t, BridgeLayerInfo info) {
    final id = layer.internallayerId.toString();
    return Row(
      children: [
        // The twirl: the layer's properties, where AE puts them. Its own
        // gesture, so opening a layer does not also select it — you often
        // want to look at one layer's values while another is selected.
        LumitTooltip(
          message: widget.open ? l10n.tipHideProperties : l10n.tipProperties,
          child: _ownClick(GestureDetector(
            key: ValueKey<String>('tl-twirl-$id'),
            behavior: HitTestBehavior.opaque,
            onTap: widget.onToggleOpen,
            child: SizedBox(
              width: 16,
              height: t.density.laneRow,
              child: Center(
                child: TwirlTurn(
                  open: widget.open,
                  child: glyph.LumitIcon(
                    widget.open ? LumitIcons.collapse : LumitIcons.expand,
                    size: iconSize,
                    colour: widget.open ? t.textPrimary : t.textMuted,
                  ),
                ),
              ),
            ),
          )),
        ),
        const SizedBox(width: identityGap),
        // The layer number: **mono**, because it is a number (§7.1's rule has
        // no exceptions), muted, and in the same 18px cell the header's `#`
        // stands in. It comes **before** the label dot: the number is
        // the row's address and the dot belongs to the name it colours, which
        // is how the mockup's rows read and how they are indexed aloud.
        SizedBox(
          width: numberCellWidth,
          child: Text('${index + 1}',
              style: t.mono.copyWith(fontSize: 10, color: t.textMuted)),
        ),
        const SizedBox(width: identityGap),
        LumitTooltip(
          message: l10n.tipLabelColour,
          child: _ownClick(_labelSwatch(context, t, id, info.label)),
        ),
        // The name is also the stack handle: drag it up or down to reorder
        // the layer (docs/07 §4.7). A locked layer holds its place.
        //
        // Selection is the row's, on the pointer down — the rename's
        // double-tap holds the gesture arena open for its whole window, so
        // selecting through a tap made a plain click on the name reach the
        // Effect controls a third of a second late.
        //
        // The drag itself: a plain vertical gesture, not a `Draggable`.
        //
        // A `Draggable` carries a floating copy of the thing being moved,
        // which is why this used to show a little name label under the
        // pointer while the real row stayed behind. Both halves of the
        // table already slide, so the stack shows the move
        // truthfully on its own — the label was a second, worse answer to
        // a question already answered, and the row it named did not move.
        // The row travels; nothing floats.
        Expanded(
          child: info.switches.locked
              ? _name(t, id, info)
              : GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  supportedDevices: dragDevices,
                  onVerticalDragStart: (_) {
                    _dragTravel = 0;
                    // Carried at Full; at Minimal and None the rows stay put
                    // and a line marks the drop's place (docs/15 §8.1).
                    widget.layerDrag.lift(index,
                        carries: ThemeScope.of(context).motion.carries);
                  },
                  // The row in hand follows the pointer, and the slot it is
                  // aiming at is worked out from the same travel.
                  onVerticalDragUpdate: (d) {
                    _dragTravel += d.delta.dy;
                    widget.layerDrag.carry(widget.blockHeights, _dragTravel);
                  },
                  onVerticalDragEnd: (_) => _commitDrag(),
                  onVerticalDragCancel: () => widget.layerDrag
                      .release(widget.blockHeights, cancelled: true),
                  child: _name(t, id, info),
                ),
        ),
        // No trailing gap of its own: the seam after this cluster is the gap
        // (`outlineGap`), and a second one behind it made the name's column
        // end 4px short of every other cluster's.
      ],
    );
  }

  /// Group 3: fx · 3D · motion blur · adjustment · flow · collapse, spread
  /// across the same span the fold-out's value cells use.
  ///
  /// **The L6 arrangement** (owner's ruling; 3D and motion blur swapped on the
  /// owner's 2026-08-31 desktop testing): fx leads the column, collapse has
  /// come out of the cell it shared with flow (the old flow-or-collapse) and
  /// stands on its own at the end, and flow sits immediately left of it. A
  /// column each means a Precomp that is also retimed footage shows both, and
  /// neither has to be read off the layer's kind to know which switch it is.
  ///
  /// Three of the six cells are drawn by kind, on the same rule: a cell is
  /// there when the row can act on it, and blank otherwise. **Footage shows
  /// Flow**, a Precomp shows collapse, and the adjustment cell is drawn on every
  /// row that shows something in the Viewer, which is all of them but the four
  /// that draw nothing.
  ///
  /// **And only what the column has room for** (T4): dragged narrower it gives
  /// its cells up in [modeHideOrder]: collapse, then flow, then adjustment,
  /// leaving fx, 3D and motion blur.
  /// Whether the adjustment cell is drawn on a row of this kind: every
  /// kind that puts something in the Viewer, which is everything except the
  /// four with no picture of their own.
  ///
  /// The frontend's half of `Layer::can_adjust`, listed as the kinds that are
  /// *out* rather than the ones that are in, so a new drawing kind gets the
  /// cell by existing rather than by being remembered here.
  static bool _canAdjust(BridgeLayerKind kind) =>
      kind != BridgeLayerKind.camera &&
      kind != BridgeLayerKind.light &&
      kind != BridgeLayerKind.nullLayer &&
      kind != BridgeLayerKind.audio;

  Widget _renderCells(
      BuildContext context, BridgeLayerInfo info, double width) {
    final id = layer.internallayerId.toString();
    final switches = info.switches;
    const blank = SizedBox(width: switchCellWidth);
    Widget cell(ModeCell which) => switch (which) {
          ModeCell.fx => _switch(context, id, 'fx', LumitIcon.fx, switches.fx,
              BridgeLayerSwitch.fx,
              tip: switches.fx
                  ? l10n.switchEffectsOn
                  : l10n.switchEffectsBypassed),
          ModeCell.motionBlur => _switch(
              context,
              id,
              'mb',
              LumitIcon.motionBlur,
              switches.motionBlur,
              BridgeLayerSwitch.motionBlur,
              tip: l10n.switchMotionBlur),
          // Blank on a camera and a light: both are three-dimensional by being
          // what they are, so a switch there would be one you cannot turn off
          // (docs/impl/camera.md §1).
          ModeCell.threeD => hasThreeDSwitch(info.kind)
              ? _switch(context, id, '3d', LumitIcon.cube3d, switches.threeD,
                  BridgeLayerSwitch.threeD,
                  tip: l10n.switchThreeD)
              : blank,
          // The adjustment cell, where accepts lights used to stand. An
          // ordinary switch cell like the ones beside it: it writes
          // `BridgeLayerSwitch.adjustment`, so it inherits the plural handler
          // and applies to the whole selection.
          //
          // **On every row that shows something in the Viewer** — footage,
          // solid, precomp, text, shape, sequence and a layer born an
          // adjustment. Only the four with no picture to set aside leave it
          // empty (camera, light, null, audio), and they keep the width so the
          // pickers after it stay in one column. Drawn regardless of the row's
          // own visibility switch: what a layer *is* and whether it is being
          // shown are two answers, and hiding one must not hide the other.
          ModeCell.adjustment => _canAdjust(info.kind)
              ? _switch(context, id, 'adjust', LumitIcon.adjustment,
                  switches.adjustment, BridgeLayerSwitch.adjustment,
                  tip: switches.adjustment
                      ? l10n.tipAdjustmentOn
                      : l10n.tipAdjustmentOff)
              : blank,
          // The Flow cell: shaped exactly like a switch but writing the
          // layer's interpolation policy rather than a `BridgeLayerSwitch`,
          // because that is what flow *is* underneath ("the option surfaces
          // the policy").
          ModeCell.flow => info.kind == BridgeLayerKind.footage
              ? _switch(context, id, 'flow', LumitIcon.flow, info.flow, null,
                  tip: info.flow ? l10n.tipFlowOn : l10n.tipFlowOff,
                  onSet: (to) {
                  // A locked layer refuses, and quietly.
                  try {
                    layer.setFlowEnabled(on_: to);
                  } catch (_) {}
                  widget.onChanged();
                })
              : blank,
          ModeCell.collapse => info.kind == BridgeLayerKind.precomp
              ? _switch(context, id, 'collapse', LumitIcon.collapse,
                  switches.collapse, BridgeLayerSwitch.collapse,
                  tip: l10n.tipCollapseTransformations)
              : blank,
        };
    final shown = modeCellsFor(width);
    return SizedBox(
      width: width,
      child: Row(
        // Packed left in ordinary switch cells, exactly as group 1 is: the
        // group's remaining span belongs to the fold-out's value column,
        // not to spreading the icons across it.
        children: [
          for (final which in ModeCell.values)
            if (shown.contains(which)) cell(which),
        ],
      ),
    );
  }

  /// Group 4: matte · blend, sharing the group's width so dragging it wider
  /// widens the pickers rather than leaving space beside them.
  Widget _composeCells(
      BuildContext context, LumitTheme t, BridgeLayerInfo info, double width) {
    final (matteWidth, blendWidth) =
        composeCellWidths(width, matteToggles: widget.matteToggles);
    return Row(
      children: [
        LumitTooltip(
          message: l10n.tipMatte,
          child: MattePickerFrb(
            layer: layer,
            info: info,
            all: widget.layers,
            width: matteWidth,
            toggleRoom: widget.matteToggles,
            resting: _resting,
            onChanged: widget.onChanged,
          ),
        ),
        const SizedBox(width: cellGap),
        LumitTooltip(
          message: l10n.tipBlendMode,
          child: _blendPicker(context, t, info.blend, blendWidth),
        ),
      ],
    );
  }

  /// Group 5: the parent picker, alone in a cluster of its own so the bottom
  /// bar's Parent toggle hides it and nothing else.
  Widget _parentCell(BridgeLayerInfo info, double width) => LumitTooltip(
        message: l10n.tipParent,
        child: ParentPickerFrb(
          layer: layer,
          info: info,
          all: widget.layers,
          width: width,
          resting: _resting,
          onChanged: widget.onChanged,
        ),
      );

  /// The comp a Precomp layer draws, if it is still in the document.
  CompositionReference? _sourceComp() {
    try {
      final source = layer.getSourceItem();
      return source is ItemReference_Composition ? source.field0 : null;
    } catch (_) {
      // A layer that has gone: nothing to open, and never a crash.
      return null;
    }
  }

  /// Double-clicking a layer opens it. A **Sequence** layer opens its
  /// own view in place — its clips and their speed envelope, inside its row —
  /// because cutting is done against the beat you can see, so the
  /// music and the ruler have to stay on screen. A Precomp opens the comp it
  /// draws, the way it does in the Project panel and the Hierarchy. Every
  /// other kind has nothing to open, so the double-click renames it, the same
  /// rename `Enter` starts.
  void _openLayer() {
    final kind = widget.entry.info.kind;
    if (kind == BridgeLayerKind.sequence) {
      widget.onOpenSequence?.call();
      return;
    }
    if (kind != BridgeLayerKind.precomp) {
      widget.renameRequest.value = layer.internallayerId;
      // Asked directly as well: a request already standing for this layer
      // does not notify a second time.
      _maybeRename();
      return;
    }
    final comp = _sourceComp();
    if (comp == null) return;
    Provider.of<LumitUiState>(context, listen: false)
        .openNestedComp(layer, comp);
  }

  /// The name, or the rename editor `Enter` turns it into. Submitting commits;
  /// clicking anywhere else commits too (the field loses the row). A locked
  /// layer's name does not open the editor: lock means no edits.
  Widget _name(LumitTheme t, String id, BridgeLayerInfo info) {
    final editor = _rename;
    if (editor != null) {
      return RowRenameField(
        key: ValueKey<String>('tl-rename-$id'),
        controller: editor,
        onCommit: _commitRename,
        onCancel: _cancelRename,
      );
    }
    return GestureDetector(
      key: ValueKey<String>('tl-name-$id'),
      behavior: HitTestBehavior.opaque,
      onDoubleTap: _openLayer,
      child: SizedBox(
        height: t.density.laneRow,
        child: Align(
          alignment: Alignment.centerLeft,
          // The chosen layer's name is the one thing on its row read at full
          // strength — the mockup brightens the name, and only the name, on
          // the selected row; every other row keeps `body`.
          child: Text(info.name,
              style: widget.selected ? t.bodyPrimary : t.body,
              overflow: TextOverflow.ellipsis),
        ),
      ),
    );
  }

  /// The layer's label colour (TL2): a chip that opens the eight-colour
  /// picker. The palette is the theme's own, so no colour literal lives here.
  Widget _labelSwatch(
      BuildContext context, LumitTheme t, String id, int label) {
    return GestureDetector(
      key: ValueKey<String>('tl-label-$id'),
      behavior: HitTestBehavior.opaque,
      onTapDown: (d) async {
        final picked = await showLabelPicker(context, d.globalPosition,
            keyPrefix: 'tl-label');
        if (picked == null) return;
        // Every selected layer, the Project panel's `_setLabel` being
        // the reference: one call each, because one call is what the engine's
        // op is. A label is one of the three writes a locked layer still
        // takes, so nothing is skipped.
        for (final target in _menuTargets()) {
          target.layer.setLabel(label: picked);
        }
        widget.onChanged();
      },
      child: SizedBox(
        // 16, not the dot's 6: the swatch opens a picker, so the cell is the
        // hit target and the dot is what is drawn in the middle of
        // it. Its 5px of inset either side is also the mockup's own gap
        // between the dot and the name that follows it.
        width: 16,
        height: t.density.laneRow,
        child: Center(
          // A 6px **dot**, the mockup's own diameter. It was a 10px rounded
          // square, which read as a swatch competing with the name beside it;
          // the mockup marks the layer with a bullet, and a bullet is what a
          // label colour is for.
          child: Container(
            width: _labelDotSize,
            height: _labelDotSize,
            decoration: BoxDecoration(
              color: t.labelColour(label),
              borderRadius: BorderRadius.circular(_labelDotSize / 2),
            ),
          ),
        ),
      ),
    );
  }

  /// One switch cell: **a bare glyph** (owner, 2026-08-24). It wore a small
  /// outlined box, on the theory that a boxed target reads as a button; the
  /// drawing has no box on any switch anywhere in the outline, and five boxed
  /// marks beside five more turned two quiet columns into a grid of buttons.
  /// The cell is still [switchCellWidth] wide and still takes the whole click,
  /// so nothing about the aiming changed — only the paint.
  ///
  /// **On is `text_primary`, off is dimmer, and neither is the accent**
  /// (§3.1's accent list is closed, and the owner has ruled on this column
  /// more than once). Nor is it `animated`: that token means "this is keyed",
  /// and a motion-blur switch is not a keyframe. The drawing agrees — it lights
  /// a row switch in the same foreground it writes the chosen layer's name in.
  ///
  /// With an [offIcon] the glyph
  /// itself flips (closed eye, muted speaker, hollow circle) and keeps full
  /// strength either way; without one the off state dims, as before.
  /// [onSet] replaces the default `set_switch` write for a cell that only
  /// wears the switch's clothes — the Flow cell, whose write is the layer's
  /// interpolation policy — in which case [which] may be null.
  ///
  /// Press a cell and drag up or down the column and every row the pointer
  /// crosses takes the state the first one took, as one undo step. The same
  /// drag the effect headings' enable boxes have (`fxEnableSwitch`).
  Widget _switch(
    BuildContext context,
    String id,
    String name,
    LumitIcon? icon,
    bool on,
    BridgeLayerSwitch? which, {
    LumitIcon? offIcon,
    // Lumit's own set, where the caller passes a glyph directly:
    // [mark]/[offMark] take the place of [icon]/[offIcon] and are drawn from
    // lumit_icons.dart. The [LumitIcon] pair stays for the cells not yet
    // ported — it resolves to the same set, so this is which name the
    // caller uses, not which family draws.
    String? mark,
    String? offMark,
    String? tip,
    ValueChanged<bool>? onSet,
  }) {
    final t = ThemeScope.of(context).theme;
    final project = Provider.of<LumitState>(context, listen: false).project;
    final me = layer.internallayerId;

    // Write [to]. A press writes it to every selected layer when this row is
    // one of them. A row the pointer only crosses mid-paint takes it [alone].
    void set(bool to, {bool alone = false}) {
      if (onSet != null) return onSet(to);
      // **Every selected layer, not only this row.** This is the one choke
      // point all the switches pass through, so it is the one place the rule
      // has to be written. They all take *this* row's new state rather than
      // each flipping its own, so a column of mixed eyes comes out even, and
      // the whole click is **one** bridge call and one undo step: a Ctrl+A
      // click used to commit one edit per layer, and undoing it walked back
      // through all fifty-three.
      //
      // The engine keeps the loop's manners: a locked *sibling* silently
      // refuses its share of the batch, while the clicked row's own refusal
      // is the whole call's. A locked layer refuses every switch but its own
      // lock and shy. That refusal is quiet too: nothing commits and nothing
      // changes.
      try {
        widget.comp.setSwitchOnLayers(
          clicked: me,
          layers: alone
              ? [me]
              : [
                  for (final target in _menuTargets())
                    target.layer.internallayerId,
                ],
          switch_: which!,
          on_: to,
        );
      } catch (_) {}
      widget.onChanged();
    }

    // A click, or the start of a paint. Alt on the solo switch solos this
    // layer alone.
    void press() {
      if (which == BridgeLayerSwitch.solo &&
          HardwareKeyboard.instance.isAltPressed) {
        return _soloAlone();
      }
      set(!on);
    }

    // On is `text_primary`. Off rests at `text_disabled`, a clear step below,
    // so a row's state reads at a glance down a tall stack, and comes up to
    // `text_muted` while the pointer is over the row or the keyboard is in it,
    // so a switch is easy to find when it is wanted.
    //
    // The eye and the speaker are the exception: off, they keep `text_muted`
    // and their struck glyph at rest too. A hidden or silenced layer has to
    // be obvious.
    final loud = name == 'visible' || name == 'audible';
    final ink = on
        ? t.textPrimary
        : loud || !_resting
            ? t.textMuted
            : t.textDisabled;
    final Widget face = mark != null
        ? glyph.LumitIcon(on || offMark == null ? mark : offMark,
            size: iconSize, colour: ink)
        : lumitIcon(on || offIcon == null ? icon! : offIcon,
            size: iconSize, color: ink);
    final cell = GestureDetector(
      key: ValueKey<String>('tl-$name-$id'),
      behavior: HitTestBehavior.opaque,
      onTap: press,
      child: SizedBox(
        width: switchCellWidth,
        height: t.density.laneRow,
        // **On whole pixels, not centred** (§6.20). A 16px glyph centred in a
        // 23px row starts at 3.5, and the icons carry a half-pixel nudge of
        // their own to land their strokes on pixel centres: the two
        // halves added up, so the whole switch column drew a pixel down and
        // to the right of the grid, with the strokes smeared across it. The
        // cell is the same size and takes the same click; only the paint
        // moves, and it moves back onto the grid the nudge assumes.
        child: Align(
          alignment: Alignment.topLeft,
          child: Padding(
            padding: EdgeInsets.only(
              left: wholePixelInset(switchCellWidth, iconSize),
              top: wholePixelInset(t.density.laneRow, iconSize),
            ),
            child: face,
          ),
        ),
      ),
    );
    final painted = DragTarget<_SwitchPaint>(
      // Answered once per cell the pointer crosses, and never accepted: the
      // crossing is the act. Only the same switch on another row takes it.
      onWillAcceptWithDetails: (details) {
        final paint = details.data;
        if (paint.cell == name && paint.from != me && paint.to != on) {
          set(paint.to, alone: true);
        }
        return false;
      },
      builder: (context, _, __) => Draggable<_SwitchPaint>(
        data: (cell: name, from: me, to: !on),
        // Nothing rides under the pointer: this drag paints, it carries
        // nothing anywhere.
        feedback: const SizedBox.shrink(),
        // The undo group is what makes the whole stroke one step. No cell
        // accepts the drop, so the drag always ends as cancelled, and that
        // is where the group closes.
        onDragStarted: () {
          project?.beginUndoGroup();
          press();
        },
        onDraggableCanceled: (_, __) => project?.endUndoGroup(),
        child: cell,
      ),
    );
    return tip == null ? painted : LumitTooltip(message: tip, child: painted);
  }

  /// Alt-click on a solo switch: this layer soloed and every other layer's
  /// solo off, as one undo step.
  void _soloAlone() {
    final project = Provider.of<LumitState>(context, listen: false).project;
    final me = layer.internallayerId;
    project?.beginUndoGroup();
    try {
      widget.comp.setSwitchOnLayers(
          clicked: me,
          layers: [me],
          switch_: BridgeLayerSwitch.solo,
          on_: true);
      // Every layer in the comp, not only the rows on screen.
      widget.comp.setSwitchOnLayers(
          clicked: me,
          layers: [
            for (final e in Provider.of<LumitUiState>(context, listen: false)
                .model
                .heldLayers)
              if (e.layer.internallayerId != me) e.layer.internallayerId,
          ],
          switch_: BridgeLayerSwitch.solo,
          on_: false);
    } catch (_) {
      // A locked layer refuses its solo, and then nothing else changes.
    } finally {
      project?.endUndoGroup();
    }
    widget.onChanged();
  }

  Widget _blendPicker(
      BuildContext context, LumitTheme t, int current, double width) {
    final modes = _blendModes ??= listBlendModes();
    // Normal leads the list, and is what an index past its end falls back to.
    final value = current < modes.length ? current : 0;
    // The cell's share of its group: a dropdown that overflows its cell is a
    // layout error, not a cosmetic one, and the label ellipsises to fit.
    final picker = SizedBox(
      width: width,
      child: BareDropdown<int>(
        key: ValueKey<String>('tl-blend-${layer.internallayerId}'),
        // In an outline row, so the mockup's 16/10 face (§12A.6).
        dense: true,
        value: value,
        options: [for (var i = 0; i < modes.length; i++) i],
        label: (i) => engineLabel(modes[i]),
        onChanged: (i) {
          layer.setBlend(index: i);
          widget.onChanged();
        },
      ),
    );
    return restingPicker(t,
        resting: _resting && value == 0,
        label: modes.isEmpty ? '' : engineLabel(modes[0]),
        picker: picker);
  }

  Future<void> _showRowMenu(BuildContext context, Offset position) async {
    // A locked layer keeps Duplicate — copying is not editing — but its own
    // order and existence are held still until it is unlocked.
    final locked = widget.entry.info.switches.locked;
    final lit = widget.entry.info.switches.acceptsLights;
    final picked = await showMenuAt<String>(
      context: context,
      position: position,
      width: 190,
      rows: (close) => [
        MenuRow(
            onPressed: () => close('duplicate'),
            child: Text(l10n.menuDuplicate)),
        // **Accepts lights is a setting, and this is where it is set.**
        // It had a cell in the Modes column and left it on the owner's ruling:
        // a fifth mark in a row of switches, on something that does nothing at
        // all in a comp with no Light layers. A ticked menu entry says the same
        // thing in words, on the rows that want it, and costs the outline
        // nothing. Not gated on the lock, exactly as the switch cells are not.
        MenuRow(
          key: const ValueKey('tl-row-accepts-lights'),
          onPressed: () => close('accepts-lights'),
          child: Row(
            children: [
              menuTick(lit),
              Expanded(child: Text(l10n.switchAcceptsLights)),
            ],
          ),
        ),
        if (!locked) ...[
          if (index > 0)
            MenuRow(
                onPressed: () => close('up'), child: Text(l10n.bringForward)),
          if (index < count - 1)
            MenuRow(
                onPressed: () => close('down'), child: Text(l10n.sendBackward)),
          // In and out of the clip-editing surface, for anyone. The Vegas
          // preference decides what an *import* becomes, never
          // what a layer is allowed to be — and coming back out is
          // offered wherever going in is, so a user who tries it can
          // change their mind.
          if (widget.entry.info.kind == BridgeLayerKind.footage)
            MenuRow(
                key: const ValueKey('tl-row-to-sequence'),
                onPressed: () => close('to-sequence'),
                child: Text(l10n.menuConvertToSequenceLayer)),
          if (widget.entry.info.kind == BridgeLayerKind.sequence)
            MenuRow(
                key: const ValueKey('tl-row-from-sequence'),
                onPressed: () => close('from-sequence'),
                child: Text(l10n.menuConvertToFootageLayer)),
          // **Detach audio**: the layer's sound onto a row of its own,
          // directly below, with this row muted. Not offered on a row that is
          // already nothing but sound — there is nothing to separate it from —
          // and a row that turns out to make no sound says so in the status
          // line rather than being greyed here, because finding out costs a
          // probe of the media and a menu cannot wait for one.
          if (widget.entry.info.kind != BridgeLayerKind.audio)
            MenuRow(
                key: const ValueKey('tl-row-detach-audio'),
                onPressed: () => close('detach-audio'),
                child: Text(l10n.menuDetachAudio)),
          // **Retime's own commands** (docs/04 §12.1), on the layers that have
          // a Retime to command. A Sequence layer's maps belong to its clips
          // and are commanded from the clips' own menu in the sequence
          // view, so offering them on the row would be offering something the
          // engine is right to refuse.
          if (widget.entry.info.kind != BridgeLayerKind.sequence) ...[
            MenuRow(
                key: const ValueKey('tl-row-retime'),
                onPressed: () => close('retime'),
                child: Text(widget.entry.info.retime == null
                    ? l10n.menuEnableRetime
                    : l10n.menuDisableRetime)),
            MenuRow(
                key: const ValueKey('tl-row-stretch'),
                onPressed: () => close('stretch'),
                child: Text(l10n.menuStretch)),
            MenuRow(
                key: const ValueKey('tl-row-freeze'),
                onPressed: () => close('freeze'),
                child: Text(l10n.menuFreezeFrame)),
          ],
        ],
        // The shape — the cuts, the gaps and the ramps, with no media in
        // it — from the layer itself, so carrying a cut onto a depth pass
        // never needs either row opened first. Offered on a locked
        // layer too: copying is not editing.
        if (widget.entry.info.kind == BridgeLayerKind.sequence) ...[
          MenuRow(
              key: const ValueKey('tl-row-copy-shape'),
              onPressed: () => close('copy-shape'),
              child: Text(l10n.copySequenceShape)),
          if (!locked && sequenceShapeClipboard != null)
            MenuRow(
                key: const ValueKey('tl-row-paste-shape'),
                onPressed: () => close('paste-shape'),
                child: Text(l10n.pasteSequenceShape)),
        ],
        if (!locked) ...[
          MenuRow(onPressed: () => close('delete'), child: Text(l10n.delete)),
        ],
        // Only when there is something to clear. A layer carries markers
        // when a composition was dropped in with some; most layers
        // have none and should not be offered a command that does nothing.
        if (!locked && widget.entry.info.markers.isNotEmpty)
          MenuRow(
              key: const ValueKey('tl-row-clear-markers'),
              onPressed: () => close('clear-markers'),
              child: Text(l10n.deleteAllMarkers)),
      ],
    );
    // Every command below this line runs on the whole picked set, and
    // every one of them keeps its own `try`/`catch` so that one layer's
    // refusal - a lock, a kind that cannot do it, a row of several clips -
    // leaves the rest of the batch standing.
    final targets = _menuTargets();
    switch (picked) {
      case 'duplicate':
        // Offered on a locked layer too: copying is not editing.
        duplicateLayersFrb([for (final target in targets) target.layer]);
      case 'up' || 'down':
        final delta = picked == 'up' ? -1 : 1;
        final ids = {
          for (final target in targets) target.layer.internallayerId,
        };
        final moving = [
          for (var i = 0; i < widget.layers.length; i++)
            if (ids.contains(widget.layers[i].layer.internallayerId) &&
                !widget.layers[i].info.switches.locked)
              i,
        ];
        // Forward from the top, backward from the bottom: a layer moving one
        // place swaps with its neighbour and leaves every index past it alone,
        // so taken in this order the original indices stay true for the whole
        // batch and a block of layers keeps its shape.
        for (final i in delta < 0 ? moving : moving.reversed) {
          final to = i + delta;
          if (to < 0 || to >= widget.layers.length) continue;
          try {
            widget.layers[i].layer
                .reorder(newIndex: BigInt.from(_stackIndex(to)));
          } catch (_) {}
        }
      case 'delete':
        for (final target in targets) {
          if (target.info.switches.locked) continue;
          try {
            target.layer.delete();
          } catch (_) {}
        }
      case 'clear-markers':
        for (final target in targets) {
          // Only where there is something to clear: an empty write is still an
          // undo step.
          if (target.info.switches.locked || target.info.markers.isEmpty) {
            continue;
          }
          try {
            target.layer.setMarkers(markers: const []);
          } catch (_) {}
        }
      case 'accepts-lights':
        // This row's new state, for all of them, so a mixed set comes out even
        // — one batched call, one undo step, like the switch cells.
        try {
          widget.comp.setSwitchOnLayers(
            clicked: layer.internallayerId,
            layers: [
              for (final target in targets) target.layer.internallayerId
            ],
            switch_: BridgeLayerSwitch.acceptsLights,
            on_: !lit,
          );
        } catch (_) {}
      case 'to-sequence':
        for (final target in targets) {
          if (target.info.kind != BridgeLayerKind.footage) continue;
          if (target.info.switches.locked) continue;
          try {
            target.layer.convertToSequenced();
          } catch (_) {}
        }
      case 'from-sequence':
        // A row of several clips refuses: which one the layer would become is
        // the user's decision, not the command's, and the engine says so.
        for (final target in targets) {
          if (target.info.kind != BridgeLayerKind.sequence) continue;
          if (target.info.switches.locked) continue;
          try {
            target.layer.convertFromSequenced();
          } catch (_) {}
        }
      case 'detach-audio':
        // A layer that makes no sound refuses, and one line says so for the
        // whole batch: four silent solids should not post four notices.
        var refused = false;
        for (final target in targets) {
          if (target.info.switches.locked) continue;
          if (target.info.kind == BridgeLayerKind.audio) continue;
          try {
            await target.layer.detachAudio();
          } catch (_) {
            refused = true;
          }
        }
        if (refused && mounted) {
          Provider.of<LumitState>(this.context, listen: false)
              .postNotice(l10n.detachAudioNoSound);
        }
      case 'retime':
        for (final target in targets) {
          if (target.info.switches.locked) continue;
          try {
            target.layer.toggleRetimeProperty();
          } catch (_) {}
        }
      case 'stretch':
        // The dialogue reads the length this row has now; every other picked
        // layer is stretched by the same *speed*, which is the number the
        // question was asked in — matching their durations instead would make
        // one command mean two different things.
        final settings = widget.comp.getSettings();
        final info = widget.entry.info;
        if (!mounted) return;
        final percent = await showStretchDialogFrb(
          // The row's own context, not the one the menu was opened from: the
          // menu has already been awaited, so `mounted` is the guard that
          // applies.
          context: this.context,
          durationFrames: info.outFrame - info.inFrame,
          fps: settings.fpsNum / settings.fpsDen,
        );
        if (percent == null || !mounted) return;
        for (final target in targets) {
          if (target.info.switches.locked) continue;
          try {
            target.layer.stretch(speedPercent: percent);
          } catch (_) {}
        }
      case 'freeze':
        if (!mounted) return;
        final frame = Provider.of<LumitUiState>(this.context, listen: false)
            .playheadFrame
            .value;
        for (final target in targets) {
          if (target.info.switches.locked) continue;
          try {
            target.layer.freezeAtPlayhead(frame: frame);
          } catch (_) {}
        }
      case 'copy-shape':
        // Singular by nature: a clipboard holds one shape, and copying four
        // would mean choosing which one survives.
        try {
          sequenceShapeClipboard = layer.copySequenceShape();
        } catch (_) {}
        return; // nothing changed in the document
      case 'paste-shape':
        final shape = sequenceShapeClipboard;
        if (shape == null) return;
        for (final target in targets) {
          if (target.info.kind != BridgeLayerKind.sequence) continue;
          if (target.info.switches.locked) continue;
          try {
            target.layer.pasteSequenceShape(text: shape);
          } catch (_) {}
        }
      case _:
        return;
    }
    widget.onChanged();
  }
}
