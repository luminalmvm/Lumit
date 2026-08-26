// The Timeline's outline column: the gutter scrollbar, the group seams, the
// column headers, the key readout and the list of rows itself.
//
// Split out of timeline_panel_frb.dart; one outline row is
// timeline_outline_row_frb.dart.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:uuid/uuid.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/timeline_columns.dart';
import '../theme/theme.dart';
import '../widgets/controls.dart';
import 'graph_editor_frb.dart';
import 'graph_maths.dart';
import 'effect_param_row_frb.dart';
import 'layer_fold_frb.dart';
import 'timeline_timings.dart';
import 'timeline_metrics_frb.dart';
import 'timeline_layer_rows_frb.dart';
import 'timeline_bar_frb.dart';
import 'timeline_outline_row_frb.dart';

/// A scrollbar for a scroll view that is somewhere else in the tree.
///
/// `RawScrollbar` learns where its scrollable is from `ScrollNotification`s
/// rising through *its own* subtree. Sat in a gutter beside the scroll view,
/// it receives none — so it never repainted and the thumb was simply
/// invisible (K-192). This listens to the controller instead, which is the
/// thing it actually needs to know about, and drags it directly.
class GutterScrollbar extends StatelessWidget {
  final ScrollController controller;
  final Axis axis;
  const GutterScrollbar({super.key, 
    required this.controller,
    this.axis = Axis.vertical,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return AnimatedBuilder(
      animation: controller,
      builder: (context, _) {
        final position = positionOf(controller);
        if (position == null || !position.hasContentDimensions) {
          return const SizedBox.expand();
        }
        final viewport = position.viewportDimension;
        final range = position.maxScrollExtent;
        // Nothing overflows: no thumb, and nothing to grab at.
        if (range <= 0.5 || viewport <= 0) return const SizedBox.expand();

        return LayoutBuilder(
          builder: (context, constraints) {
            final track = axis == Axis.vertical
                ? constraints.maxHeight
                : constraints.maxWidth;
            if (track <= 0) return const SizedBox.expand();
            final extent =
                (viewport / (viewport + range) * track).clamp(20.0, track);
            final travel = track - extent;
            final offset = travel <= 0 ? 0.0 : position.pixels / range * travel;

            void dragBy(double delta) {
              if (travel <= 0) return;
              controller.jumpTo(
                  (position.pixels + delta / travel * range).clamp(0.0, range));
            }

            final thumb = MouseRegion(
              cursor: SystemMouseCursors.grab,
              child: GestureDetector(
                key: const ValueKey('tl-gutter-thumb'),
                behavior: HitTestBehavior.opaque,
                onVerticalDragUpdate:
                    axis == Axis.vertical ? (d) => dragBy(d.delta.dy) : null,
                onHorizontalDragUpdate:
                    axis == Axis.horizontal ? (d) => dragBy(d.delta.dx) : null,
                child: Container(
                  // The 3 is along the thumb's length only: its thickness is
                  // set by the Positioned below, at [scrollbarThickness] in
                  // both directions.
                  margin: axis == Axis.horizontal
                      ? const EdgeInsets.symmetric(horizontal: 3)
                      : const EdgeInsets.symmetric(vertical: 3),
                  decoration: BoxDecoration(
                    // `surface_4`, the mockup's own thumb value: a raised
                    // block, not a rule. `hairline_strong` is for lines.
                    color: t.surface4,
                    borderRadius: BorderRadius.circular(3),
                  ),
                ),
              ),
            );

            return Stack(
              children: [
                axis == Axis.vertical
                    ? Positioned(
                        top: offset,
                        // The same 7 the horizontal bar wears (§6.15),
                        // centred in the gutter — not the gutter's width less
                        // a margin, which came out a pixel thinner than the
                        // bar under the same view.
                        // Floored, because the 12px gutter cannot centre a 7
                        // on whole pixels and a block edge on a half pixel is
                        // a smear: half a pixel off centre is invisible, a
                        // soft edge is not.
                        left: wholePixelInset(
                            constraints.maxWidth, scrollbarThickness),
                        width: scrollbarThickness,
                        height: extent,
                        child: thumb)
                    : Positioned(
                        left: offset,
                        // The mockups' 7px bar (K-451, docs/15 §12A.6),
                        // centred in whatever bar carries it — not a thumb
                        // grown from the bar's own height, which is how it
                        // came out 14 and read as a second toolbar.
                        top: wholePixelInset(
                            constraints.maxHeight, scrollbarThickness),
                        height: scrollbarThickness,
                        width: extent,
                        child: thumb),
              ],
            );
          },
        );
      },
    );
  }
}

/// The seam between adjacent column groups, in a row: plain space of exactly
/// [groupDividerWidth]. The header's rule is enough to read the grouping by;
/// repeating it down every row of a tall stack is noise. The width matches
/// the header's seam so the two stay column-aligned.
const Widget rowSeam = SizedBox(width: groupDividerWidth);

/// The header's seam: the hairline that names the grouping, and the handle
/// that resizes the group to its left (docs/07 §4.2). Everything else keeps
/// its width, so a drag here widens or narrows the whole outline.
class _GroupSeam extends StatefulWidget {
  /// Null for a group whose width is fixed: the rule still draws, but the seam
  /// is not a handle and does not offer a resize cursor.
  final ValueChanged<double>? onResize;
  const _GroupSeam({super.key, required this.onResize});

  @override
  State<_GroupSeam> createState() => _GroupSeamState();
}

/// **Staged, and committed once on release** (owner, desktop testing: the seam
/// drag lagged).
///
/// A column width is pure view state — nothing about it reaches the document —
/// so the lag was never a write. It was the *rebuild*: every pointer move
/// called back into the panel, which called `setState` on the whole Timeline,
/// which rebuilt every outline row, every picker, every lane and every bar for
/// one hairline moving a pixel. Sixty of those a second is sixty rebuilds of
/// the panel a second.
///
/// So the gesture holds its own total and draws its own answer — a line where
/// the seam would land, which is the only thing that moves under the pointer —
/// and the panel hears about it once, when the hand lets go. That is the rule
/// every other drag in this panel already follows: a bar stages in
/// [BarDragPreview], a key in `_KeyLaneState`, a work-area edge in the ruler's
/// own `_dragFrame`.
class _GroupSeamState extends State<_GroupSeam> {
  /// How far the seam has been dragged since the button went down, or null
  /// when no drag is in flight.
  double? _staged;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final rule = SizedBox(
      width: groupDividerWidth,
      child: Center(
        child: Container(width: 1, height: 14, color: t.hairlineStrong),
      ),
    );
    final resize = widget.onResize;
    if (resize == null) return rule;
    final staged = _staged;
    return MouseRegion(
      cursor: SystemMouseCursors.resizeColumn,
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onHorizontalDragStart: (_) => setState(() => _staged = 0),
        onHorizontalDragUpdate: (d) =>
            setState(() => _staged = (_staged ?? 0) + d.delta.dx),
        onHorizontalDragEnd: (_) {
          final moved = _staged;
          setState(() => _staged = null);
          if (moved != null && moved != 0) resize(moved);
        },
        onHorizontalDragCancel: () => setState(() => _staged = null),
        child: Stack(
          clipBehavior: Clip.none,
          children: [
            rule,
            // Where the seam will land, drawn only while the hand is on it.
            // `accent` because this *is* the one thing being aimed at, and it
            // is gone the moment the button lifts.
            if (staged != null)
              Positioned(
                left: groupDividerWidth / 2 + staged,
                top: 0,
                bottom: 0,
                child: IgnorePointer(
                  child: Container(
                    key: const ValueKey('tl-seam-preview'),
                    width: 1,
                    color: t.accent,
                  ),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

/// The column-group header (docs/07 §4.2): **one kicker word per column**,
/// grouped into the clusters, each cluster draggable as a unit to reorder them.
///
/// Words, not icons (§12A.1, K-451). A column header names a container, and
/// §7.1 sets every container label as a kicker; a row of small glyphs made the
/// reader work out what each column *was* from the same marks the cells below
/// already carry. The switch cells still wear their icons — those are the
/// controls; this is the legend.
///
/// The second of the outline's two chrome rows, and so
/// `t.density.timelineHeaderRow` — **23** under Regular, 18 under Compact
/// (K-512). A shade shorter than the row above it, because that row is aimed
/// at and this one is mostly read.
class ColumnHeader extends StatelessWidget {
  final List<TimelineGroup> order;
  final Map<TimelineGroup, double> widths;

  /// Whether the compose group's width carries the matte mode toggles' room
  /// (K-463): the same answer the rows are given, so the MATTE and BLEND
  /// kickers stand over the cells below them either way.
  final bool matteToggles;
  final void Function(TimelineGroup dragged, TimelineGroup target) onReorder;

  /// A seam dragged: widen (or narrow) the group on its left by `delta`.
  final void Function(TimelineGroup group, double delta) onResize;

  const ColumnHeader({super.key, 
    required this.order,
    required this.widths,
    required this.matteToggles,
    required this.onReorder,
    required this.onResize,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Container(
      height: t.density.timelineHeaderRow,
      // The panel's own ground with a rule under it, like the timecode row
      // above — the mockup draws no raised strip here.
      decoration: BoxDecoration(
        color: t.surface1,
        border: Border(bottom: BorderSide(color: t.hairline)),
      ),
      padding: const EdgeInsets.only(left: 10, right: 8),
      child: Row(
        children: [
          for (var i = 0; i < order.length; i++) ...[
            // The seam resizes the group it follows, which is the one the eye
            // reads it as belonging to.
            if (i > 0)
              _GroupSeam(
                key: ValueKey<String>('tl-seam-${order[i - 1].name}'),
                // Null for a fixed-width group: the rule is still drawn, so
                // the grouping reads, but there is nothing to take hold of.
                onResize: groupIsFixedWidth(order[i - 1])
                    ? null
                    : (delta) => onResize(order[i - 1], delta),
              ),
            _draggable(context, t, order[i]),
          ],
        ],
      ),
    );
  }

  Widget _draggable(BuildContext context, LumitTheme t, TimelineGroup group) {
    final content = SizedBox(
      width: widths[group],
      child: _cells(t, group, widths[group] ?? 0),
    );
    return DragTarget<TimelineGroup>(
      onWillAcceptWithDetails: (d) => d.data != group,
      onAcceptWithDetails: (d) => onReorder(d.data, group),
      builder: (context, candidate, _) => Draggable<TimelineGroup>(
        key: ValueKey<String>('tl-colgroup-${group.name}'),
        data: group,
        // **The tag rides the cursor** (owner, desktop testing). Flutter's
        // default anchors the feedback to where the pointer was inside the
        // *child* — and the child here is a column header a couple of hundred
        // pixels wide, while the tag is one short word. Grabbing a header
        // anywhere but its left edge therefore drew the word back at the
        // header's own x, far from the hand carrying it, which read as the
        // drag having been dropped. Anchored at the pointer, the word is
        // under the finger wherever the header was taken hold of.
        dragAnchorStrategy: pointerDragAnchorStrategy,
        feedback: Container(
          height: t.density.timelineHeaderRow,
          padding: const EdgeInsets.symmetric(horizontal: 8),
          color: t.surface2,
          child: Center(
            child: Text(_labelOf(group), style: t.small),
          ),
        ),
        childWhenDragging: Opacity(opacity: 0.4, child: content),
        child: Container(
          color: candidate.isEmpty ? null : t.accent.withValues(alpha: 0.18),
          child: content,
        ),
      ),
    );
  }

  String _labelOf(TimelineGroup group) => columnGroupLabel(group);

  /// The header cells, in the same widths the rows use, so each word stands
  /// over its column. Indicators only — clicking a header does nothing; the
  /// switches live on the rows (docs/07 §4.2). Each carries a hover hint
  /// naming its column, which for a truncated word is how the whole of it is
  /// still read.
  Widget _cells(LumitTheme t, TimelineGroup group, double width) {
    /// One kicker over a column, left-aligned on the column's own edge.
    Widget title(String text, String tip, double cellWidth,
            {double inset = 0}) =>
        SizedBox(
          width: cellWidth,
          child: LumitTooltip(
            message: tip,
            child: Align(
              alignment: Alignment.centerLeft,
              child: Padding(
                padding: EdgeInsets.only(left: inset),
                child: Text(text.toUpperCase(),
                    style: t.kicker,
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis),
              ),
            ),
          ),
        );
    return switch (group) {
      // One word over the whole cluster rather than five marks over five
      // cells: the switches themselves say which is which, down the rows.
      TimelineGroup.switches =>
        title(l10n.columnSwitches, l10n.columnSwitches, width),
      TimelineGroup.identity => Row(
          children: [
            // The twirl has no heading of its own, so the blank before the `#`
            // is the row's own twirl slot and gap (K-461).
            const SizedBox(width: 16 + identityGap),
            title(l10n.columnNumber, l10n.columnNumber, numberCellWidth),
            const SizedBox(width: identityGap),
            // **The dot column is headed by the set's Label glyph** (owner,
            // 2026-08-24). The mockup's header names the columns it can name
            // in a word — SWITCHES, #, LAYER, MATTE, BLEND, PARENT, MS — and
            // leaves this one blank, which left the dots looking like a strip
            // of colour nobody had asked for. A word here would be a fourth
            // kicker inside the identity cluster and wider than the 16 the
            // column is; the glyph is exactly the column's width, and it is
            // the one heading in the row that cannot be mistaken for the mark
            // below it, because the marks below it are colours.
            //
            // Muted like every other kicker in this row, and centred, because
            // the dots it stands over are centred in their own cell.
            SizedBox(
              key: const ValueKey<String>('tl-colhead-label'),
              width: 16,
              child: LumitTooltip(
                message: l10n.tipLabelColour,
                child: Center(
                  child: glyph.LumitIcon(
                    LumitIcons.label,
                    size: 12,
                    colour: t.textMuted,
                    semanticLabel: l10n.tipLabelColour,
                  ),
                ),
              ),
            ),
            Expanded(
              child: Text(l10n.columnLayer.toUpperCase(),
                  style: t.kicker,
                  maxLines: 1,
                  overflow: TextOverflow.ellipsis),
            ),
          ],
        ),
      TimelineGroup.render => title(l10n.columnModes, l10n.columnModes, width),
      // The render-time column's header is its switch — see timeline_timings.
      TimelineGroup.timings => const TimingsHeaderCell(),
      TimelineGroup.compose => () {
          final (matte, blend) =
              composeCellWidths(width, matteToggles: matteToggles);
          // The compose titles carry the dropdown's own text inset, so each
          // sits directly over the text in the cell below it.
          return Row(
            children: [
              title(l10n.columnMatte, l10n.tipMatte, matte,
                  inset: dropdownTextInset),
              const SizedBox(width: cellGap),
              title(l10n.columnBlend, l10n.tipBlendMode, blend,
                  inset: dropdownTextInset),
            ],
          );
        }(),
      TimelineGroup.parent => title(l10n.columnParent, l10n.tipParent, width,
          inset: dropdownTextInset),
    };
  }
}
/// A number written the way the Timeline writes numbers: whole numbers plain, everything else
/// to two places — the drawing's own `960, 540`, `100`, `1.60`, `0.60`.
String keysNumberText(double v) =>
    v == v.roundToDouble() ? v.round().toString() : v.toStringAsFixed(2);

/// What rides beside a graph channel's value, or null for a number with no
/// unit — the readout row's answer to §12A.3's rule.
///
/// An effect parameter's unit is its **declaration's** (K-443), never its id.
/// A transform axis has no declaration to ask, so it is read off the property
/// itself: the two scales and opacity are per cent, the three rotations are
/// degrees, and everything else is a distance — pixels at composition size
/// (K-419).
String? graphChannelUnit(GraphChannel channel) {
  if (channel.param case final param?) return unitRiderText(param.unit);
  if (channel.retime) return l10n.unitSymbolSeconds;
  if (channel.maskValue case final value?) {
    return switch (value) {
      MaskValue.opacity => l10n.unitSymbolPercent,
      MaskValue.feather ||
      MaskValue.vertexFeather ||
      MaskValue.expansion =>
        l10n.unitSymbolPx,
      MaskValue.path => null,
    };
  }
  return switch (channel.prop) {
    null => null,
    BridgeTransformProp.opacity ||
    BridgeTransformProp.scaleX ||
    BridgeTransformProp.scaleY =>
      l10n.unitSymbolPercent,
    BridgeTransformProp.rotation ||
    BridgeTransformProp.rotationX ||
    BridgeTransformProp.rotationY =>
      l10n.unitSymbolDegrees,
    _ => l10n.unitSymbolPx,
  };
}

/// The **Key readout row** (§3.3, `GraphMode.dc.html`): pinned at the foot of
/// Graph mode's outline while exactly one key is selected, reading
/// `KEY f<frame> <value><unit>` and offering that key's two influences as
/// editable wells.
///
/// **One key only**, because that is what it can say: two or more are a block,
/// and a block's readout is its own badge. It draws from the selection alone,
/// so it arrives and leaves with the key rather than being another thing to
/// dismiss (P1) — and it draws *nothing* when there is no single key, keeping
/// the outline's foot the height it was.
///
/// The wells commit through the same write the tangent handles make: a side
/// becomes a bezier at its current speed and the influence asked for, so
/// typing 33 into **In** and dragging the handle to a third of the span are
/// the same edit and one undo step.
class KeyReadoutRow extends StatelessWidget {
  final List<GraphChannel> channels;
  final Set<String> selectedKeys;
  final double fps;
  final VoidCallback onChanged;

  const KeyReadoutRow({super.key, 
    required this.channels,
    required this.selectedKeys,
    required this.fps,
    required this.onChanged,
  });

  /// The one selected key, as its channel and its index — or null when the
  /// selection is not exactly one key that still exists.
  (GraphChannel, int)? get _one {
    if (selectedKeys.length != 1) return null;
    final id = selectedKeys.first;
    final hash = id.lastIndexOf('#');
    if (hash < 0) return null;
    final index = int.tryParse(id.substring(hash + 1));
    if (index == null) return null;
    final channelId = id.substring(0, hash);
    for (final channel in channels) {
      if (channel.id != channelId) continue;
      if (index < 0 || index >= channel.keys.length) return null;
      return (channel, index);
    }
    return null;
  }

  /// Write one side's influence, keeping the speed it already reads at — the
  /// tangent handle's own commit, reached by typing instead of by dragging.
  void _setInfluence(GraphChannel channel, int index, bool isOut, double v) {
    final keys = channel.keys;
    final side = sideWithInfluence(keys, index, isOut, v);
    commitChannelEdits({
      channel: BridgeScalar.keyframed([
        for (var i = 0; i < keys.length; i++)
          if (i == index)
            BridgeKeyframe(
              time: keys[i].time,
              value: keys[i].value,
              interpIn: isOut ? keys[i].interpIn : side,
              interpOut: isOut ? side : keys[i].interpOut,
            )
          else
            keys[i],
      ]),
    });
    onChanged();
  }

  Widget _well(String name, double percent, ValueChanged<num> set) => SizedBox(
        width: 40,
        child: DragValueField(
          key: ValueKey<String>(name),
          value: percent,
          min: 0,
          max: 100,
          decimals: 0,
          onChanged: set,
        ),
      );

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final one = _one;
    if (one == null) return const SizedBox.shrink();
    final (channel, index) = one;
    final key = channel.keys[index];
    final frame = (rationalSeconds(key.time) * (fps <= 0 ? 1 : fps)).round();
    final value = key.value;
    return Container(
      key: const ValueKey('tl-graph-key-readout'),
      height: t.density.secondaryRow,
      decoration: BoxDecoration(
        color: t.surface1,
        border: Border(top: BorderSide(color: t.hairline)),
      ),
      padding: const EdgeInsets.only(left: 10, right: 8),
      child: Row(
        children: [
          Text(l10n.graphKeyKicker.toUpperCase(), style: t.kicker),
          const SizedBox(width: 8),
          Text(l10n.graphKeyFrame(frame),
              style: t.mono.copyWith(fontSize: 10, color: t.textPrimary)),
          const SizedBox(width: 8),
          Text(keysNumberText(value),
              style: t.mono.copyWith(fontSize: 10, color: t.textPrimary)),
          if (graphChannelUnit(channel) case final String unit) ...[
            const SizedBox(width: 3),
            Text(unit, style: t.mono.copyWith(fontSize: 9, color: t.textMuted)),
          ],
          const Spacer(),
          Text(l10n.graphEaseIn.toUpperCase(), style: t.kicker),
          const SizedBox(width: 6),
          _well(
              'tl-graph-key-in',
              (sideInfluence(key.interpIn) * 100).roundToDouble(),
              (v) => _setInfluence(channel, index, false, v.toDouble())),
          const SizedBox(width: 3),
          Text(l10n.unitSymbolPercent,
              style: t.mono.copyWith(fontSize: 9, color: t.textMuted)),
          const SizedBox(width: 8),
          Text(l10n.graphEaseOut.toUpperCase(), style: t.kicker),
          const SizedBox(width: 6),
          _well(
              'tl-graph-key-out',
              (sideInfluence(key.interpOut) * 100).roundToDouble(),
              (v) => _setInfluence(channel, index, true, v.toDouble())),
          const SizedBox(width: 3),
          Text(l10n.unitSymbolPercent,
              style: t.mono.copyWith(fontSize: 9, color: t.textMuted)),
        ],
      ),
    );
  }
}

/// The left column: one row per layer, with its switches and columns.
class Outline extends StatelessWidget {
  final CompositionReference comp;

  /// The layers as the panel decided them — the same [LayerRow] list the lane
  /// area draws from, so a row's fold-out, its open Sequence view and its
  /// height are one answer rather than two that agree.
  final List<LayerRow> rows;

  /// The column groups in their current order and at their current widths
  /// (docs/07 §4.2) — rows draw their cells to match the header's.
  final List<TimelineGroup> groupOrder;
  final Map<TimelineGroup, double> widths;

  /// Whether the compose group's width carries the matte mode toggles' room —
  /// the panel's one answer for the whole outline (K-463).
  final bool matteToggles;

  /// The whole selection as ids (K-217), worked out once by the panel: a row
  /// asking "am I selected?" is then one set lookup rather than a walk of the
  /// list per row per paint.
  final Set<UuidValue> selectedIds;
  final String? highlighted;

  /// The selected properties' fold paths, in selection order: each is a
  /// curve in the graph, its row draws selected, and every row containing
  /// one highlights (docs/07 §4.3, §5).
  final List<String> selectedProperties;

  /// Each selected path's graph line colours, for tinting its label.
  final Map<String, List<Color>> graphColours;
  final ValueChanged<String> onSelectProperty;
  final ValueChanged<String> onEditProperty;

  /// Open or close a Sequence layer's view (K-248).
  final void Function(BridgeLayerEntry entry)? onOpenSequence;
  final ValueChanged<String> onToggle;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final ValueChanged<LayerReference> onSelect;
  final ValueChanged<String> onHighlight;
  final VoidCallback onChanged;

  /// The drag in flight and the block heights it slides by — the panel's, so
  /// the lanes are working from the same two values (K-208).
  final ValueNotifier<LayerDrag?> layerDrag;
  final List<double> blockHeights;

  /// The layer `Enter` has just asked to rename (K-243).
  final ValueNotifier<UuidValue?> renameRequest;

  const Outline({super.key, 
    required this.comp,
    required this.rows,
    required this.groupOrder,
    required this.widths,
    required this.matteToggles,
    required this.selectedIds,
    required this.highlighted,
    required this.selectedProperties,
    required this.graphColours,
    required this.onSelectProperty,
    required this.onEditProperty,
    this.onOpenSequence,
    required this.onToggle,
    required this.playheadFrame,
    required this.onSeek,
    required this.onSelect,
    required this.onHighlight,
    required this.onChanged,
    required this.layerDrag,
    required this.blockHeights,
    required this.renameRequest,
  });

  @override
  Widget build(BuildContext context) {
    // The column geometry is the same for every row, so it is worked out once
    // here rather than once per fold row of every layer.
    final valueColumn = valueColumnFor(groupOrder, widths);
    final timingsColumn = timingsColumnFor(groupOrder, widths);
    final baseIndent = identityStart(groupOrder, widths);
    // The layer entries, for the parent picker's menu — every layer is on
    // offer as a parent, and they come from the row list rather than from a
    // second list handed in beside it.
    final layers = [for (final row in rows) row.entry];
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        for (var i = 0; i < rows.length; i++)
          LayerDragSlide(
            drag: layerDrag,
            heights: blockHeights,
            index: i,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                OutlineRow(
                  key: ValueKey<String>('tl-row-${rows[i].id}'),
                  comp: comp,
                  entry: rows[i].entry,
                  onOpenSequence: () => onOpenSequence?.call(rows[i].entry),
                  layers: layers,
                  groupOrder: groupOrder,
                  widths: widths,
                  matteToggles: matteToggles,
                  index: i,
                  count: rows.length,
                  // A local compare, not a bridge call: both ids already sit here.
                  selected:
                      selectedIds.contains(rows[i].entry.layer.internallayerId),
                  // A layer marks itself when its fold was last touched, and when
                  // a selected property is one of its own (docs/07 §4.3).
                  highlighted: highlighted == rows[i].id ||
                      selectedProperties.any((p) => isUnderPath(rows[i].id, p)),
                  open: rows[i].open,
                  hasAudio: rows[i].hasAudio,
                  hasPicture: rows[i].hasPicture,
                  onToggleOpen: () => onToggle(rows[i].id),
                  onSelect: () => onSelect(rows[i].entry.layer),
                  onChanged: onChanged,
                  layerDrag: layerDrag,
                  renameRequest: renameRequest,
                  blockHeights: blockHeights,
                ),
                // The room the lanes draw an open sequence view in (K-248). The
                // outline has nothing to put here — the clips and their envelope are
                // the lane's to draw — but it must leave exactly the same gap, or
                // every row below this one sits at a different height on the two
                // sides of the Timeline and the halves stop lining up. Both sides
                // ask the same [LayerRow], so the gap and the view cannot be
                // opened by one half and not the other.
                if (rows[i].sequenceExtra != null)
                  SizedBox(
                    key: ValueKey<String>('tl-seq-room-${rows[i].id}'),
                    height: rows[i].sequenceExtra,
                  ),
                // The fold-out, from the same list the lanes leave room for.
                for (final row in rows[i].drawnRows)
                  // A raw pointer listener, not a gesture: touching a sub-item
                  // highlights its layer, and it must never fight the row's own
                  // taps and drags for the gesture arena.
                  Listener(
                    onPointerDown: (_) => onHighlight(rows[i].id),
                    child: FoldRow(
                      // Named after the property it draws, so a test — and a
                      // reveal — can find one row among a stack of them.
                      key: ValueKey<String>(
                          'tl-keys-prop-${foldRowPath(rows[i].id, row)}'),
                      comp: comp,
                      layer: rows[i].entry.layer,
                      row: row,
                      valueColumn: valueColumn,
                      timingsColumn: timingsColumn,
                      baseIndent: baseIndent,
                      path: foldRowPath(rows[i].id, row),
                      selectedProperties: selectedProperties,
                      graphColours: graphColours,
                      onSelectProperty: onSelectProperty,
                      onEditProperty: onEditProperty,
                      playheadFrame: playheadFrame,
                      onSeek: onSeek,
                      onToggle: onToggle,
                      onChanged: onChanged,
                      locked: rows[i].entry.info.switches.locked,
                    ),
                  ),
              ],
            ),
          ),
      ],
    );
  }
}
