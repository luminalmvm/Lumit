// One row of the Timeline's outline: the number, the label dot, the name, the
// switches, the blend mode and the parent picker.
//
// Split out of timeline_panel_frb.dart. It is one class doing one job — a row
// — and stayed whole.

import 'package:flutter/gestures.dart';
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
import '../shell/stretch_dialog_frb.dart';
import '../state/timeline_columns.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'timeline_extras_frb.dart';
import 'sequence_view_frb.dart';
import 'timeline_timings.dart';
import 'timeline_metrics_frb.dart';
import 'timeline_outline_frb.dart';

/// The blend-mode names, fetched once per session: the list is static for the
/// life of the process, and every outline row was re-fetching it per rebuild.
List<String>? _blendModes;

/// The label-colour dot beside a layer's name — the mockup's own 6px bullet
/// (K-451), drawn round whatever the shape is: this is a colour swatch, not a
/// control, and Sharp's square corners have nothing to say about a bullet.
const double _labelDotSize = 6;

class OutlineRow extends StatefulWidget {
  final CompositionReference comp;
  final BridgeLayerEntry entry;

  /// Open or close this layer's sequence view (K-248) — what a double-click
  /// on a Sequence layer means, where on other kinds it opens the source.
  final VoidCallback? onOpenSequence;

  /// Every layer in the comp, for the parent picker's menu — from the same
  /// read model, so offering them costs nothing.
  final List<BridgeLayerEntry> layers;

  /// The column groups in their current order, and their current widths
  /// (docs/07 §4.2).
  final List<TimelineGroup> groupOrder;
  final Map<TimelineGroup, double> widths;

  /// Whether the matte column carries its mode toggles' room (K-463) — the
  /// panel's answer for the whole comp, not this row's: a row with no matte
  /// still leaves the slot when a row above it has one.
  final bool matteToggles;
  final int index;
  final int count;
  final bool selected;

  /// A sub-item of this layer was last touched — drawn a shade dimmer than
  /// selection, so the two states read apart at a glance.
  final bool highlighted;
  final bool open;

  /// What this layer can do (K-435), so the switches column offers only that:
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
  /// the outline move with it (K-208).
  final ValueNotifier<LayerDrag?> layerDrag;

  /// The layer the panel has just been asked to rename (`Enter`, K-243), or
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
    final drag = widget.layerDrag.value;
    widget.layerDrag.value = null;
    if (drag == null || drag.from == drag.to) return;
    widget.layers[drag.from].layer.reorder(newIndex: BigInt.from(drag.to));
    widget.onChanged();
  }

  LayerReference get layer => widget.entry.layer;
  int get index => widget.index;
  int get count => widget.count;

  /// What a command invoked on this row acts on (K-523): **the whole selection
  /// when this row is part of it, and this row alone when it is not**.
  ///
  /// The same rule the Project panel's `_targets` states — a right-click on an
  /// unpicked row is about that row, and everything else is about what is
  /// picked. Returned in stack order, from the panel's own layer list, so a
  /// reorder can count on the order it reads.
  ///
  /// Read from the shell rather than passed down, and only ever from a
  /// handler: a row's build must not ask what is selected beyond the `selected`
  /// flag it is already given (K-184).
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
  }

  @override
  void dispose() {
    widget.renameRequest.removeListener(_maybeRename);
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

  /// Escape: shut the editor and rename nothing (K-323). Shares the closing
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
    // ZERO bridge calls: everything this row draws is in the read model
    // (K-184).
    final info = widget.entry.info;

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
        // ground behind these rows deselects on tap (K-203), and a row that
        // entered no tap into the arena let the ground win and throw away the
        // selection the pointer-down had just made.
        onTap: () {},
        onSecondaryTapDown: (d) => _showRowMenu(context, d.globalPosition),
        child: Container(
          // No drop line: the rows themselves move to where they would land,
          // so a line marking the same slot said it twice.
          child: _rowBody(context, t, info),
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

  /// Mark [child]'s clicks as the control's own, not the row's.
  Widget _ownClick(Widget child) =>
      Listener(onPointerDown: (_) => _claimed = true, child: child);

  Widget _rowBody(BuildContext context, LumitTheme t, BridgeLayerInfo info) {
    return Container(
        key: ValueKey<String>('tl-rowbody-${layer.internallayerId}'),
        height: t.density.laneRow,
        decoration: BoxDecoration(
          // Selected is the brighter of the two states; a highlight (this
          // layer's fold-out was last touched) is the same surface at half
          // strength, so they read apart at a glance.
          color: widget.selected
              ? t.selectionFill
              : widget.highlighted
                  ? t.selectionFill.withValues(alpha: 0.45)
                  : null,
          // No seam of its own: K-192's overlay draws the seams for the whole
          // outline, and a border here drew a *second* line a fraction of a
          // pixel from it — the overlay is phased by the scroll offset, which
          // a trackpad leaves fractional, so the two lines pulled apart as the
          // table scrolled and the outline's rows read a hair taller than the
          // lanes beside them.
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
                // Only the identity group is the layer itself — its name and
                // its number are what you click to choose it. The other three
                // are controls: hiding a layer, or picking its blend mode, is
                // not choosing it, and those cells have never selected.
                child: switch (widget.groupOrder[i]) {
                  TimelineGroup.identity => _identityCells(context, t, info),
                  TimelineGroup.switches =>
                    _ownClick(_switchCells(context, t, info)),
                  TimelineGroup.render =>
                    _ownClick(_renderCells(context, info)),
                  TimelineGroup.compose => _ownClick(_composeCells(context, t,
                      info, widget.widths[TimelineGroup.compose] ?? 0)),
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
        ));
  }

  /// Group 1: visibility · audio · solo · lock · shy. The first two swap
  /// their glyph when off — a closed eye, a muted speaker — rather than only
  /// dimming, so the off state reads at a glance.
  ///
  /// **Only what the layer can do** (K-435). The eye is drawn for a layer with
  /// a picture, the speaker for a layer with sound — so an Audio layer has no
  /// eye, and a solid, a title, a shape or an image-only clip has no speaker.
  /// A control that does nothing when clicked is worse than no control: you
  /// have to click it to find out. Each keeps its cell's width either way, so
  /// the switches stay in their columns down the stack and the ones a row does
  /// have sit where the eye reads for them.
  Widget _switchCells(
      BuildContext context, LumitTheme t, BridgeLayerInfo info) {
    final id = layer.internallayerId.toString();
    final switches = info.switches;
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        if (widget.hasPicture)
          _switch(context, id, 'visible', null, switches.visible,
              BridgeLayerSwitch.visible,
              mark: LumitIcons.visible,
              offMark: LumitIcons.hidden,
              tip: switches.visible ? l10n.switchVisible : l10n.switchHidden)
        else
          SizedBox(width: switchCellWidth, height: t.density.laneRow),
        if (widget.hasAudio)
          _switch(context, id, 'audible', null, switches.audible,
              BridgeLayerSwitch.audible,
              mark: LumitIcons.audio,
              offMark: LumitIcons.muted,
              tip: switches.audible ? l10n.switchAudible : l10n.switchMuted)
        else
          SizedBox(width: switchCellWidth, height: t.density.laneRow),
        // A ringed dot, dimmed until soloed — the set has one solo mark, so
        // this pair is told apart by strength rather than by shape.
        _switch(
            context, id, 'solo', null, switches.solo, BridgeLayerSwitch.solo,
            mark: LumitIcons.solo,
            offMark: LumitIcons.solo,
            tip: switches.solo ? l10n.switchSoloed : l10n.switchSolo),
        _switch(context, id, 'locked', null, switches.locked,
            BridgeLayerSwitch.locked,
            mark: LumitIcons.lock,
            offMark: LumitIcons.unlocked,
            tip: switches.locked ? l10n.switchLocked : l10n.switchLock),
        _switch(context, id, 'shy', LumitIcon.shyHidden, switches.shy,
            BridgeLayerSwitch.shy,
            offIcon: LumitIcon.shy,
            tip: switches.shy ? l10n.switchShy : l10n.switchMarkShy),
        // Guide (K-497), the cell beside shy that docs/07 §4.2 names for it.
        // **Drawn on every row**, unlike the two kind-gated cells in the Modes
        // column: any layer can be reference-only — a match photograph, a
        // grid, an animatic — so there is no kind the mark would do nothing
        // on. The two strengths are the column's own: lit `text_primary` while
        // the layer is a guide, resting at `text_muted` when it is not.
        _switch(
            context, id, 'guide', null, switches.guide, BridgeLayerSwitch.guide,
            mark: LumitIcons.guide,
            tip: switches.guide ? l10n.switchGuide : l10n.switchMarkGuide),
      ],
    );
  }

  /// Group 2: twirl · layer number · label dot · name (K-461 — the mockup's
  /// own order; the dot and the number used to stand the other way round).
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
                child: glyph.LumitIcon(
                  widget.open ? LumitIcons.collapse : LumitIcons.expand,
                  size: iconSize,
                  colour: widget.open ? t.textPrimary : t.textMuted,
                ),
              ),
            ),
          )),
        ),
        const SizedBox(width: identityGap),
        // The layer number: **mono**, because it is a number (§7.1's rule has
        // no exceptions), muted, and in the same 18px cell the header's `#`
        // stands in. It comes **before** the label dot (K-461): the number is
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
        // table already slide (K-208), so the stack shows the move
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
                    widget.layerDrag.value = LayerDrag(index, index);
                  },
                  onVerticalDragUpdate: (d) {
                    _dragTravel += d.delta.dy;
                    final to = layerDragTarget(
                        widget.blockHeights, index, _dragTravel);
                    final drag = widget.layerDrag.value;
                    if (drag?.to == to && drag?.from == index) return;
                    widget.layerDrag.value = LayerDrag(index, to);
                  },
                  onVerticalDragEnd: (_) => _commitDrag(),
                  onVerticalDragCancel: () => widget.layerDrag.value = null,
                  child: _name(t, id, info),
                ),
        ),
        // No trailing gap of its own: the seam after this cluster is the gap
        // (`outlineGap`), and a second one behind it made the name's column
        // end 4px short of every other cluster's.
      ],
    );
  }

  /// Group 3: flow (collapse on a Precomp) · fx · motion blur · 3D ·
  /// adjustment, spread across the same span the fold-out's value cells use.
  ///
  /// Two of the five cells are drawn by kind, on the same rule: a cell is there
  /// when the row can act on it, and blank otherwise. The flow slot is the
  /// spec's flow-or-collapse cell (K-168) — a Precomp shows its collapse
  /// switch, **footage shows its Flow switch** (K-088/K-331), everything else
  /// leaves it empty; the adjustment cell (K-537) is drawn on every row that
  /// shows something in the Viewer, which is all of them but the four that
  /// draw nothing.
  /// Whether the adjustment cell is drawn on a row of this kind (K-537): every
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

  Widget _renderCells(BuildContext context, BridgeLayerInfo info) {
    final id = layer.internallayerId.toString();
    final switches = info.switches;
    return SizedBox(
      width: renderGroupWidth,
      child: Row(
        children: [
          // Packed left in ordinary switch cells, exactly as group 1 is: the
          // group's remaining span belongs to the fold-out's value column,
          // not to spreading four icons across it.
          if (info.kind == BridgeLayerKind.precomp)
            _switch(context, id, 'collapse', LumitIcon.collapse,
                switches.collapse, BridgeLayerSwitch.collapse,
                tip: l10n.tipCollapseTransformations)
          else if (info.kind == BridgeLayerKind.footage)
            // The Flow cell: shaped exactly like a switch but writing the
            // layer's interpolation policy rather than a `BridgeLayerSwitch`,
            // because that is what flow *is* underneath (K-088: "the option
            // surfaces the policy").
            _switch(context, id, 'flow', LumitIcon.flow, info.flow, null,
                tip: info.flow ? l10n.tipFlowOn : l10n.tipFlowOff, onTap: () {
              layer.setFlowEnabled(on_: !info.flow);
              widget.onChanged();
            })
          else
            const SizedBox(width: switchCellWidth),
          _switch(context, id, 'fx', LumitIcon.fx, switches.fx,
              BridgeLayerSwitch.fx,
              tip: switches.fx
                  ? l10n.switchEffectsOn
                  : l10n.switchEffectsBypassed),
          _switch(context, id, 'mb', LumitIcon.motionBlur, switches.motionBlur,
              BridgeLayerSwitch.motionBlur,
              tip: l10n.switchMotionBlur),
          _switch(context, id, '3d', LumitIcon.cube3d, switches.threeD,
              BridgeLayerSwitch.threeD,
              tip: l10n.switchThreeD),
          // The adjustment cell (K-537), where accepts lights used to stand
          // (K-483). An ordinary switch cell like the three before it: it
          // writes `BridgeLayerSwitch.adjustment`, so it inherits the plural
          // handler and applies to the whole selection (K-523).
          //
          // **On every row that shows something in the Viewer** — footage,
          // solid, precomp, text, shape, sequence and a layer born an
          // adjustment. Only the four with no picture to set aside leave it
          // empty (camera, light, null, audio), and they keep the width so the
          // pickers after it stay in one column. Drawn regardless of the row's
          // own visibility switch: what a layer *is* and whether it is being
          // shown are two answers, and hiding one must not hide the other.
          if (_canAdjust(info.kind))
            _switch(context, id, 'adjust', LumitIcon.adjustment,
                switches.adjustment, BridgeLayerSwitch.adjustment,
                tip: switches.adjustment
                    ? l10n.tipAdjustmentOn
                    : l10n.tipAdjustmentOff)
          else
            const SizedBox(width: switchCellWidth),
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

  /// Double-clicking a layer opens it (K-243). A **Sequence** layer opens its
  /// own view in place — its clips and their speed envelope, inside its row
  /// (K-248) — because cutting is done against the beat you can see, so the
  /// music and the ruler have to stay on screen. A Precomp opens the comp it
  /// draws, the way it does in the Project panel and the Hierarchy; every
  /// other kind will open in a Viewer of its own once there is one to open,
  /// and until then does nothing. It no longer renames — `Enter` does that.
  void _openLayer() {
    if (widget.entry.info.kind == BridgeLayerKind.sequence) {
      widget.onOpenSequence?.call();
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
      return HouseTextField(
        key: ValueKey<String>('tl-rename-$id'),
        controller: editor,
        autofocus: true,
        onSubmitted: (_) => _commitRename(),
        // Clicking anywhere else finishes the edit and keeps what was typed.
        // It used to leave the field open and lose the change (K-243).
        onTapOutside: _commitRename,
        onCancelled: _cancelRename,
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
        final picked = await showLumitPopup<int>(
          context: context,
          position: d.globalPosition,
          builder: (close) => FloatSurface(
            child: Padding(
              padding: const EdgeInsets.all(6),
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  for (var i = 0; i < LumitTheme.labelCount; i++)
                    GestureDetector(
                      key: ValueKey<String>('tl-label-chip-$i'),
                      onTap: () => close(i),
                      child: Container(
                        width: 14,
                        height: 14,
                        margin: const EdgeInsets.all(2),
                        decoration: BoxDecoration(
                          color: t.labelColour(i),
                          borderRadius:
                              BorderRadius.circular(t.tokens.controlRadius),
                        ),
                      ),
                    ),
                ],
              ),
            ),
          ),
        );
        if (picked == null) return;
        // Every selected layer (K-523), the Project panel's `_setLabel` being
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
        // hit target (K-452) and the dot is what is drawn in the middle of
        // it. Its 5px of inset either side is also the mockup's own gap
        // between the dot and the name that follows it.
        width: 16,
        height: t.density.laneRow,
        child: Center(
          // A 6px **dot** (K-451: the mockup's own diameter). It was a 10px
          // rounded square, which read as a swatch competing with the name
          // beside it; the mockup marks the layer with a bullet, and a bullet
          // is what a label colour is for.
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
  /// **On is `text_primary`, off is `text_muted`, and neither is the accent**
  /// (§3.1's accent list is closed, and the owner has ruled on this column
  /// more than once). Nor is it `animated`: that token means "this is keyed",
  /// and a motion-blur switch is not a keyframe. The drawing agrees — it lights
  /// a row switch in the same foreground it writes the chosen layer's name in.
  ///
  /// With an [offIcon] the glyph
  /// itself flips (closed eye, muted speaker, hollow circle) and keeps full
  /// strength either way; without one the off state dims, as before.
  /// [onTap] replaces the default `set_switch` write for a cell that only
  /// wears the switch's clothes — the Flow cell, whose write is the layer's
  /// interpolation policy — in which case [which] may be null.
  Widget _switch(
    BuildContext context,
    String id,
    String name,
    LumitIcon? icon,
    bool on,
    BridgeLayerSwitch? which, {
    LumitIcon? offIcon,
    // Lumit's own set (K-440), where the caller passes a glyph directly:
    // [mark]/[offMark] take the place of [icon]/[offIcon] and are drawn from
    // lumit_icons.dart. The [LumitIcon] pair stays for the cells not yet
    // ported — it resolves to the same set (K-611), so this is which name the
    // caller uses, not which family draws.
    String? mark,
    String? offMark,
    String? tip,
    VoidCallback? onTap,
  }) {
    final t = ThemeScope.of(context).theme;
    // **Two strengths, one rule** — the drawing lights every row switch at
    // `text_primary` and rests it at `text_muted`, and has no third reading.
    // A switch whose glyph does not flip used to rest at `text_disabled`
    // instead, on the theory that a shape that says nothing needs the dimmer
    // off; with the boxed faces gone the colour is the whole of the state, and
    // two strengths that a reader can tell apart beat three that shade into
    // one another.
    final ink = on ? t.textPrimary : t.textMuted;
    final Widget face = mark != null
        ? glyph.LumitIcon(on || offMark == null ? mark : offMark,
            size: iconSize, colour: ink)
        : lumitIcon(on || offIcon == null ? icon! : offIcon,
            size: iconSize, color: ink);
    final cell = GestureDetector(
      key: ValueKey<String>('tl-$name-$id'),
      behavior: HitTestBehavior.opaque,
      onTap: onTap ??
          () {
            // **Every selected layer, not only this row** (K-523) — this is
            // the one choke point all six switches pass through, so it is the
            // one place the rule has to be written. They all take *this*
            // row's new state rather than each flipping its own, so a column
            // of mixed eyes comes out even.
            //
            // The clicked row keeps its unguarded call: a locked layer refuses
            // every switch but its own lock and shy, and what that refusal
            // should look like is the outline's own open question. A locked
            // *sibling* only refuses its share of the batch.
            for (final target in _menuTargets()) {
              if (target.layer.internallayerId == layer.internallayerId) {
                target.layer.setSwitch(switch_: which!, on_: !on);
              } else {
                try {
                  target.layer.setSwitch(switch_: which!, on_: !on);
                } catch (_) {}
              }
            }
            widget.onChanged();
          },
      child: SizedBox(
        width: switchCellWidth,
        height: t.density.laneRow,
        // **On whole pixels, not centred** (§6.20). A 16px glyph centred in a
        // 23px row starts at 3.5, and the icons carry a half-pixel nudge of
        // their own to land their strokes on pixel centres (K-456): the two
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
    return tip == null ? cell : LumitTooltip(message: tip, child: cell);
  }

  Widget _blendPicker(
      BuildContext context, LumitTheme t, int current, double width) {
    final modes = _blendModes ??= listBlendModes();
    // The cell's share of its group: a dropdown that overflows its cell is a
    // layout error, not a cosmetic one, and the label ellipsises to fit.
    return SizedBox(
      width: width,
      child: BareDropdown<int>(
        key: ValueKey<String>('tl-blend-${layer.internallayerId}'),
        // In an outline row, so the mockup's 16/10 face (§12A.6, K-451).
        dense: true,
        value: current < modes.length ? current : 0,
        options: [for (var i = 0; i < modes.length; i++) i],
        label: (i) => engineLabel(modes[i]),
        onChanged: (i) {
          layer.setBlend(index: i);
          widget.onChanged();
        },
      ),
    );
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
        // **Accepts lights (K-361) is a setting, and this is where it is set.**
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
          // preference decides what an *import* becomes (K-246), never
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
          // **Retime's own commands** (docs/04 §12.1), on the layers that have
          // a Retime to command. A Sequence layer's maps belong to its clips
          // (K-075) and are commanded from the clips' own menu in the sequence
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
        // never needs either row opened first (K-248). Offered on a locked
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
        // when a composition was dropped in with some (K-254); most layers
        // have none and should not be offered a command that does nothing.
        if (!locked && widget.entry.info.markers.isNotEmpty)
          MenuRow(
              key: const ValueKey('tl-row-clear-markers'),
              onPressed: () => close('clear-markers'),
              child: Text(l10n.deleteAllMarkers)),
      ],
    );
    // Every command below this line runs on the whole picked set (K-523), and
    // every one of them keeps its own `try`/`catch` so that one layer's
    // refusal - a lock, a kind that cannot do it, a row of several clips -
    // leaves the rest of the batch standing.
    final targets = _menuTargets();
    switch (picked) {
      case 'duplicate':
        // Offered on a locked layer too: copying is not editing.
        for (final target in targets) {
          try {
            target.layer.duplicate();
          } catch (_) {}
        }
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
            widget.layers[i].layer.reorder(newIndex: BigInt.from(to));
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
        // This row's new state, for all of them, so a mixed set comes out even.
        for (final target in targets) {
          try {
            target.layer
                .setSwitch(switch_: BridgeLayerSwitch.acceptsLights, on_: !lit);
          } catch (_) {}
        }
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
