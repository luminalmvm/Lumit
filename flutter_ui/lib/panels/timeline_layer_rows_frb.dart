// The Timeline's fold rows: the drop target over an empty panel, the fold row
// itself (one twirled-open property or group), and the plain, flow and volume
// rows it builds.
//
// Split out of timeline_panel_frb.dart.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';
import '../icons/icons.dart';
import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/strings.dart';
import '../state/comp_time.dart';
import '../state/drag_payloads.dart';
import '../state/timeline_columns.dart';
import '../widgets/controls.dart';
import 'placeholder.dart';
import 'timeline_extras_frb.dart';
import 'effect_param_row_frb.dart';
import 'keyframe_controls_frb.dart';
import 'layer_fold_frb.dart';
import 'package:lumit_flutter/src/rust/api/retime.dart';
import 'flow_rows_frb.dart';
import 'fx_section.dart';
import 'timeline_timings.dart';
import 'transform_rows_frb.dart';
import 'timeline_mask_rows_frb.dart';
import 'timeline_shape_rows_frb.dart';
import 'timeline_retime_row_frb.dart';

/// The Timeline with no composition open: the placeholder, and a drop target
/// over it.
///
/// Dropping footage here asks for the new comp's settings — opened on the
/// media's own size, rate and length — and each dropped item lands in it as a
/// layer; dropping a composition simply opens that one. Without this the
/// panel was a dead end: the drag lifted, showed its feedback, and dropped
/// into nothing.
class EmptyTimelineDrop extends StatelessWidget {
  final LumitState state;
  const EmptyTimelineDrop({super.key, required this.state});

  @override
  Widget build(BuildContext context) {
    return DragTarget<Object>(
      onWillAcceptWithDetails: (details) =>
          details.data is FootageDragData || details.data is CompDragData,
      onAcceptWithDetails: (details) async {
        switch (details.data) {
          case FootageDragData(:final footage):
            final comp = await state.newComposition(context, footage: footage);
            if (comp == null || !context.mounted) return;
            Provider.of<LumitUiState>(context, listen: false)
                .setSelectedComp(comp);
          case CompDragData(comp: final dropped):
            Provider.of<LumitUiState>(context, listen: false)
                .setSelectedComp(dropped);
        }
      },
      builder: (context, candidate, _) => Container(
        foregroundDecoration: candidate.isEmpty
            ? null
            : BoxDecoration(
                border: Border.all(
                    color: ThemeScope.of(context).theme.accent, width: 2),
              ),
        child: PlaceholderPanel(
          icon: LumitIcon.comp,
          title: l10n.panelTimeline,
          hint: l10n.timelineEmpty,
        ),
      ),
    );
  }
}

/// One row of a layer's fold-out, in the outline.
///
/// A heading draws its own twirl; a property row draws the same controls the
/// Effect controls panel does, at exactly one lane's height so the two halves of
/// the table stay in step.
class FoldRow extends StatelessWidget {
  final CompositionReference comp;
  final LayerReference layer;
  final LayerFoldRow row;

  /// Where the value cells go, so they line up under the render-switch group
  /// whatever order the groups are dragged into (docs/07 §4.3).
  final ValueColumn valueColumn;

  /// Where the render-time readout goes, so an effect's measured cost sits
  /// under the same header its layer's does (docs/13 §7.1).
  final ValueColumn timingsColumn;

  /// Where the identity group starts in the current order — the fold-out
  /// hangs off the layer's own twirl, so a group's twirl sits just inside it
  /// rather than at the row's far left.
  final double baseIndent;

  /// This row's path, and the selected properties' — the row draws itself
  /// selected when it is among them, and highlighted when a selection sits
  /// *under* it (an effect's heading while one of its parameters is picked).
  final String path;
  final List<String> selectedProperties;

  /// Each selected path's graph line colours, one per axis — the label text
  /// takes them so the outline names its curves (docs/07 §5).
  final Map<String, List<Color>> graphColours;
  final ValueChanged<String> onSelectProperty;

  /// Editing a value (or keying) selects the property too, without the
  /// click-gesture modifiers.
  final ValueChanged<String> onEditProperty;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final ValueChanged<String> onToggle;
  final VoidCallback onChanged;

  /// Whether the layer this row belongs to is locked (K-291). A locked layer's
  /// rows are still *read* — the numbers are what the document holds and the
  /// curves still draw — but nothing on them can be touched.
  final bool locked;

  const FoldRow({
    super.key,
    required this.comp,
    required this.layer,
    required this.row,
    required this.valueColumn,
    required this.timingsColumn,
    required this.baseIndent,
    required this.path,
    required this.selectedProperties,
    required this.graphColours,
    required this.onSelectProperty,
    required this.onEditProperty,
    required this.playheadFrame,
    required this.onSeek,
    required this.onToggle,
    required this.onChanged,
    required this.locked,
  });

  @override
  Widget build(BuildContext context) {
    // Just inside the layer's twirl, then one step per level, so a parameter
    // sits under its effect and an effect under Effects.
    final indent = baseIndent + 8.0 + (row.depth - 1) * 12.0;

    // No per-row change listener: the whole panel repaints from the read model
    // when anything commits (K-184), so the numbers shown are the document's.
    final t = ThemeScope.of(context).theme;
    final selected = selectedProperties.contains(path);
    final contains =
        !selected && selectedProperties.any((p) => isUnderPath(path, p));
    // Selection rides on the property's *name* (docs/07 §4.3) — and on any
    // press that *acts* on the row (K-334): the stopwatch, the ◄ ◆ ►
    // navigator, a value drag. Touching a row's controls IS choosing it, and
    // before this a value drag on an unselected row moved a curve the graph
    // was not even showing. Pointer-down rather than tap, so the selection —
    // and with it the graph channel — exists before the first drag tick. A
    // modified press is left to the label's own Ctrl/Shift semantics, and a
    // group heading keeps its pick-and-twirl click (K-300).
    final picks = row is! FoldGroupRow && row is! FoldWaveformRow;
    // **And the row must WIN that press, not merely see it** (K-343). The
    // ground under the outline clears the selection on tap, and its comment
    // has always said "a switch or a property still wins its own tap in the
    // arena" — which was true only of rows carrying a gesture recogniser. A
    // `Listener` is not one: it watches pointers and never competes. So a mask
    // row lit up on the press and went out again on the release, when the
    // ground took the tap nothing had claimed. This claims it, for every
    // picking row, which is what makes them all behave alike.
    //
    // Empty `onTap`, because the selecting is done on pointer-down above:
    // being in the arena at all is the whole job. The row's own controls sit
    // inside and win their taps ahead of it.
    final row_ = Listener(
      // **The whole row takes the press, not just the parts with a widget in
      // them** (K-343). A `Listener` defers to its children by default, and a
      // property row is mostly empty space — so a click beside the label never
      // reached this at all, fell through to the outline behind, and *cleared*
      // the selection instead of making one. Worst on a mask's Path row, which
      // has no value field and so is almost all empty. A heading keeps
      // defer-to-child: its own detector owns the click (K-300).
      behavior: picks ? HitTestBehavior.opaque : HitTestBehavior.deferToChild,
      onPointerDown: !picks
          ? null
          : (_) {
              final keys = HardwareKeyboard.instance;
              if (keys.isControlPressed ||
                  keys.isMetaPressed ||
                  keys.isShiftPressed) {
                return;
              }
              onEditProperty(path);
            },
      child: Container(
        height: t.density.laneRow,
        // Selected is the full surface; a row that merely *contains* the
        // selection — the effect heading over a picked parameter — is the
        // same at half strength, exactly as a layer row marks itself.
        decoration: BoxDecoration(
          color: selected
              ? t.selectionFill
              : contains
                  ? t.selectionFill.withValues(alpha: 0.45)
                  : null,
        ),
        // **The trailing inset is the layer rows' own** (`outlineRowTrailing`).
        // A fold row's value cells and its render-time reading sit in the
        // columns the layer rows set, and both are laid out from the right —
        // so a row that kept less space at its trailing end than a layer row
        // pushed every one of them out of column. It kept a bare 4, which was
        // the layer rows' padding before the redesign made it 8 and before the
        // header's inset added the 2 on top; the effect headings' milliseconds
        // stood 6px right of the layer totals they add up to.
        padding: EdgeInsets.only(left: indent, right: outlineRowTrailing),
        // A locked layer's rows are read-only, not hidden (K-291): the numbers
        // are still the document's and the curves still draw, but nothing on the
        // row can be touched. The engine refuses the edit anyway — this is what
        // stops the interface offering a gesture that would only be refused.
        //
        // A *group* row is exempt: twirling one open is navigation, not editing,
        // and a locked layer that could not be looked inside would be worse than
        // one that can.
        child: locked && row is! FoldGroupRow && row is! FoldWaveformRow
            ? AbsorbPointer(
                child: Opacity(opacity: 0.5, child: _control(context)),
              )
            : _control(context),
      ),
    );
    return picks
        ? GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: () {},
            child: row_,
          )
        : row_;
  }

  /// Copy the effect this heading names (K-275) — or, when it is one of
  /// several picked, all of them (K-300). The Timeline's half of the pair, the
  /// Effect controls panel's heading carrying the other.
  void _effectMenu(BuildContext context, Offset at, String effectId) {
    showMenuAt<void>(
      context: context,
      position: at,
      width: 190,
      rows: (close) => [
        MenuRow(
          key: ValueKey<String>('tl-fx-menu-copy-$effectId'),
          onPressed: () {
            close(null);
            final ui = Provider.of<LumitUiState>(context, listen: false);
            try {
              ui.copyEffectsToClipboard(layer.copyEffects(
                effects:
                    ui.effectsToCopy(layer, UuidValue.fromString(effectId)),
              ));
            } catch (_) {
              // The effect went away between the menu opening and this row
              // being chosen; the clipboard keeps whatever it had.
            }
          },
          child: Text(l10n.copyEffect),
        ),
      ],
    );
  }

  Widget _control(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return switch (row) {
      FoldWaveformRow() => const SizedBox.shrink(),
      FoldGroupRow(:final path, :final label, :final open) => GestureDetector(
          key: ValueKey<String>('tl-group-$path'),
          behavior: HitTestBehavior.opaque,
          // **A heading is picked as well as twirled** (K-300). Until this, a
          // click on one only twirled, so an effect could not be selected here
          // at all and Copy had nothing to take. A plain click still opens the
          // heading, because that is what it has always done and the fold is
          // how the outline is navigated; a *modified* click only picks, so
          // Ctrl- and Shift-clicking a run of effects does not flap every one
          // of them open on the way past.
          onTap: () {
            onSelectProperty(path);
            if (!isModifiedClick) onToggle(path);
          },
          // An *effect's* heading offers to copy the picked effects (K-275,
          // K-300). The other headings — Transform, Effects, Masks, Audio — are
          // groupings rather than things that can be copied, and
          // `effectIdOfPath` is what tells them apart: only an effect's path
          // carries an id.
          onSecondaryTapUp: effectIdOfPath(path) == null
              ? null
              : (details) => _effectMenu(
                    context,
                    details.globalPosition,
                    effectIdOfPath(path)!,
                  ),
          child: Row(
            children: [
              GestureDetector(
                key: ValueKey<String>('tl-twirl-$path'),
                behavior: HitTestBehavior.opaque,
                onTap: () => onToggle(path),
                child: SizedBox(
                  // Wider than the glyph: the twirl is now the only way to open
                  // a heading, so it has to be worth aiming at.
                  width: iconSize + 6,
                  child: glyph.LumitIcon(
                    open ? LumitIcons.collapse : LumitIcons.expand,
                    size: iconSize,
                    colour: open ? t.textPrimary : t.textMuted,
                  ),
                ),
              ),
              const SizedBox(width: 4),
              // An effect's own heading carries what that effect cost, in the
              // render-time column with the layer totals (docs/13 §7.1). Every
              // other heading — Transform, Effects, Audio — is a grouping
              // rather than a thing that renders, so it carries nothing.
              //
              // **Expanded, and no Spacer.** A `Flexible` label beside a
              // `Spacer` splits the free space between them, which put the
              // number halfway across the row instead of in the column: two
              // flex children share, they do not queue. One Expanded label
              // takes the space, and the cell that follows lands hard right —
              // where the layer rows' numbers are.
              if (effectIdOfPath(path) case final String effectId
                  when timingsColumn.width > 0) ...[
                Expanded(
                  child: Text(label,
                      style: t.body, overflow: TextOverflow.ellipsis),
                ),
                Padding(
                  padding: EdgeInsets.only(right: timingsColumn.rightInset),
                  child: SizedBox(
                    width: timingsColumn.width,
                    child: TimingsCell(effectId: effectId),
                  ),
                ),
              ] else
                Flexible(
                  child: Text(label,
                      style: t.body, overflow: TextOverflow.ellipsis),
                ),
            ],
          ),
        ),
      FoldTransformRow(:final group, :final transform) => TransformRowFrb(
          comp: comp,
          layer: layer,
          transform: transform,
          group: group,
          playheadFrame: playheadFrame,
          onSeek: onSeek,
          onChanged: () {
            onEditProperty(path);
            onChanged();
          },
          keyPrefix: 'tl-tf',
          rowPadding: EdgeInsets.zero,
          valueColumn: valueColumn,
          onLabelTap: () => onSelectProperty(path),
          graphColours: graphColours[path],
        ),
      FoldEffectParamRow() => _TimelineParamRow(
          comp: comp,
          layer: layer,
          row: row as FoldEffectParamRow,
          valueColumn: valueColumn,
          playheadFrame: playheadFrame,
          onSeek: onSeek,
          onChanged: () {
            onEditProperty(path);
            onChanged();
          },
          onLabelTap: () => onSelectProperty(path),
          graphColour: graphColours[path]?.firstOrNull,
        ),
      FoldFlowRow() => _FlowRow(
          onLabelTap: () => onSelectProperty(path),
          comp: comp,
          layer: layer,
          row: row as FoldFlowRow,
          valueColumn: valueColumn,
          playheadFrame: playheadFrame,
          onSeek: onSeek,
          onChanged: () {
            onEditProperty(path);
            onChanged();
          },
        ),
      FoldVolumeRow(:final scalar) => _VolumeRow(
          comp: comp,
          layer: layer,
          scalar: scalar,
          valueColumn: valueColumn,
          playheadFrame: playheadFrame,
          onSeek: onSeek,
          onChanged: () {
            onEditProperty(path);
            onChanged();
          },
        ),
      FoldRetimeRow(:final scalar) => RetimeRow(
          comp: comp,
          layer: layer,
          scalar: scalar,
          valueColumn: valueColumn,
          playheadFrame: playheadFrame,
          onSeek: onSeek,
          onChanged: onChanged,
          onLabelTap: () => onSelectProperty(path),
        ),
      FoldMaskRow(:final mask) => MaskRow(
          comp: comp,
          layer: layer,
          mask: mask,
          valueColumn: valueColumn,
          onChanged: () {
            onEditProperty(path);
            onChanged();
          },
          onLabelTap: () => onSelectProperty(path),
        ),
      FoldMaskValueRow(:final mask, :final value, :final vertex) =>
        MaskValueRow(
          onLabelTap: () => onSelectProperty(path),
          comp: comp,
          layer: layer,
          mask: mask,
          value: value,
          vertex: vertex,
          valueColumn: valueColumn,
          playheadFrame: playheadFrame,
          onSeek: onSeek,
          onChanged: () {
            onEditProperty(path);
            onChanged();
          },
        ),
      FoldShapeRow(:final item) => ShapeItemRow(
          comp: comp,
          layer: layer,
          item: item,
          valueColumn: valueColumn,
          onChanged: onChanged,
        ),
      FoldShapePaintRow(:final item, :final which) => ShapePaintRow(
          layer: layer,
          item: item,
          which: which,
          valueColumn: valueColumn,
          onChanged: onChanged,
        ),
      FoldShapeValueRow(:final item, :final value) => ShapeValueRow(
          comp: comp,
          layer: layer,
          item: item,
          value: value,
          valueColumn: valueColumn,
          playheadFrame: playheadFrame,
          onSeek: onSeek,
          onChanged: onChanged,
        ),
      FoldAnimatorValueRow(:final index, :final animator, :final value) =>
        AnimatorValueRow(
          comp: comp,
          layer: layer,
          index: index,
          animator: animator,
          value: value,
          valueColumn: valueColumn,
          playheadFrame: playheadFrame,
          onSeek: onSeek,
          onChanged: () {
            onEditProperty(path);
            onChanged();
          },
        ),
      FoldStrokeValueRow(:final stroke, :final value) => StrokeValueRow(
          comp: comp,
          layer: layer,
          stroke: stroke,
          value: value,
          valueColumn: valueColumn,
          playheadFrame: playheadFrame,
          onSeek: onSeek,
          onChanged: onChanged,
        ),
      FoldStrokeRow(:final stroke) => StrokeRow(
          comp: comp,
          layer: layer,
          stroke: stroke,
          valueColumn: valueColumn,
          onChanged: onChanged,
        ),
    };
  }
}

/// One effect parameter in the Timeline. It owns the staging for its own drag,
/// which is all the state a single row needs — no stack is read to *display*:
/// the value rides in on the fold row from the read model (K-184), and a drag
/// in flight overlays its staged value on top.
class _TimelineParamRow extends StatefulWidget {
  final CompositionReference comp;
  final LayerReference layer;
  final FoldEffectParamRow row;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;
  final VoidCallback? onLabelTap;
  final Color? graphColour;

  const _TimelineParamRow({
    required this.comp,
    required this.layer,
    required this.row,
    required this.valueColumn,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
    this.onLabelTap,
    this.graphColour,
  });

  @override
  State<_TimelineParamRow> createState() => _TimelineParamRowState();
}

class _TimelineParamRowState extends State<_TimelineParamRow> {
  final EffectStackEditor _editor = EffectStackEditor();

  @override
  void dispose() {
    // The editor's preview throttle owns a timer; a row unmounted mid-drag
    // (a twirl shutting, a layer deleted) must not leave it ticking.
    _editor.clear();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final row = widget.row;
    final ui = Provider.of<LumitUiState>(context, listen: false);
    return EffectParamRowFrb(
      key: ValueKey<String>('tl-fx-${row.info.id}-${row.param.id}'),
      effectId: row.info.id,
      param: row.param,
      valueColumn: widget.valueColumn,
      // One lane tall, like every other fold row: the card's own vertical
      // padding on top of that clipped the fields.
      rowPadding: EdgeInsets.zero,
      // The staged value while a drag is in flight, the document's otherwise.
      value: _editor.stagedValue(row.info.id, row.param.id) ?? row.value,
      siblings: {for (final v in row.info.values) v.id: v.value},
      // The wire deciding this parameter, read once per revision by the panel
      // (K-471, K-627): the mark replaces the row's stopwatch and the field
      // stops taking a drag.
      driven: row.driven,
      comp: widget.comp,
      ownerLayerId: widget.layer.internallayerId,
      ownerLayers: ui.model.layers,
      playheadFrame: widget.playheadFrame,
      onSeek: widget.onSeek,
      onLabelTap: widget.onLabelTap,
      graphColour: widget.graphColour,
      onWrite: (effect, param, value) {
        _editor.write(widget.layer, effect, param, value);
        setState(() {});
        widget.onChanged();
      },
      onLive: (effect, param, value) => setState(() {
        _editor.live(widget.comp, widget.layer, effect, param, value,
            frame: ui.playheadFrame.value, scale: ui.viewerScale);
      }),
    );
  }
}

/// The Audio group's one row: the layer's Volume, in dB.
/// One control of the Flow group in the Timeline fold-out (K-088, K-331).
///
/// Every kind but the Input rate writes the whole group in one op, so the row
/// needs no state of its own: read, change one field, write it back. The Input
/// rate is a keyframeable scalar, so it alone carries the stopwatch and the
/// navigator — the same shape the Retime and Volume rows use.
class _FlowRow extends StatelessWidget {
  final CompositionReference comp;
  final LayerReference layer;
  final FoldFlowRow row;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  /// Clicking the name selects the property and its keys (K-500 §2.1) — the
  /// handle every other property row has and this one did not.
  final VoidCallback? onLabelTap;

  const _FlowRow({
    required this.comp,
    required this.layer,
    required this.row,
    required this.valueColumn,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
    this.onLabelTap,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // Read once per document revision by the panel, never here (K-184). The
    // fallback covers a caller that supplied none, which the panel never is.
    final p = row.params ?? layer.getFlowParams();

    void write(BridgeFlowParams next) {
      layer.setFlowParams(params: next);
      onChanged();
    }

    final control = switch (row.kind) {
      FlowRowKind.resolution =>
        _choice('flow-resolution', flowResolutionOptions, p.resolution, (v) {
          write(flowParamsWith(p, resolution: v));
        }),
      FlowRowKind.detail =>
        _choice('flow-detail', flowDetailOptions, p.detail, (v) {
          write(flowParamsWith(p, detail: v));
        }),
      FlowRowKind.occlusion =>
        _choice('flow-occlusion', flowOcclusionOptions, p.occlusion, (v) {
          write(flowParamsWith(p, occlusion: v));
        }),
      FlowRowKind.fallback =>
        _choice('flow-fallback', flowFallbackOptions, p.fallback, (v) {
          write(flowParamsWith(p, fallback: v));
        }),
      FlowRowKind.smoothness => SizedBox(
          width: valueColumn.width,
          child: DragValueField(
            key: const ValueKey('flow-smoothness'),
            value: p.smoothness,
            min: 0,
            max: 100,
            onChanged: (v) =>
                write(flowParamsWith(p, smoothness: v.toDouble())),
          ),
        ),
      FlowRowKind.hudGuard => HouseCheckbox(
          key: const ValueKey('flow-hud-guard'),
          value: p.hudGuard,
          onChanged: (v) => write(flowParamsWith(p, hudGuard: v)),
        ),
      FlowRowKind.always => HouseCheckbox(
          key: const ValueKey('flow-always'),
          value: p.always,
          onChanged: (v) => write(flowParamsWith(p, always: v)),
        ),
      FlowRowKind.inputRate => _inputRate(),
    };

    return Row(
      children: [
        if (row.kind == FlowRowKind.inputRate)
          KeyframeControlsFrb(
            scalars: [row.rate!],
            comp: comp,
            playheadFrame: playheadFrame,
            onSeek: onSeek,
            rowKey: 'tl-flow-rate',
            onWrite: (next) {
              layer.setFlowInputRate(value: next.single);
              onChanged();
            },
          )
        else
          const SizedBox(width: fxKeyframeGutter),
        const SizedBox(width: 4),
        Expanded(
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: onLabelTap,
            child: Row(children: [
              Flexible(child: Text(row.kind.label, style: t.body)),
            ]),
          ),
        ),
        SizedBox(width: valueColumn.width, child: control),
      ],
    );
  }

  Widget _choice(
    String keyName,
    List<String> options,
    int value,
    ValueChanged<int> onChanged,
  ) =>
      SizedBox(
        width: valueColumn.width,
        child: FlowChoice(
          keyName: keyName,
          options: options,
          value: value,
          onChanged: onChanged,
        ),
      );

  /// The conform rate: a typed value with the cadence presets beside it, and
  /// keyframes, so a cut that changes cadence partway can be followed.
  Widget _inputRate() {
    final rate = row.rate!;
    final shown = switch (rate) {
      BridgeScalar_Static(:final field0) => field0,
      // An expression is sampled engine-side too, so it needs no case of its
      // own here — `sampledScalar` is the one place either is evaluated.
      BridgeScalar_Keyframed() ||
      BridgeScalar_Expression() =>
        sampledScalar(rate, timeOfFrame(comp, playheadFrame)),
    };
    return FlowRateControl(
      shown: shown,
      fieldWidth: (valueColumn.width * 0.45).clamp(48, 90),
      gap: 4,
      onRate: (fps) {
        layer.setFlowInputRate(
          value: scalarWithValueAt(rate, fps, comp, playheadFrame),
        );
        onChanged();
      },
    );
  }
}

class _VolumeRow extends StatefulWidget {
  final CompositionReference comp;
  final LayerReference layer;

  /// The Volume scalar, read once per document revision by the panel and
  /// riding in on the fold row (K-184).
  final BridgeScalar? scalar;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  const _VolumeRow({
    required this.comp,
    required this.layer,
    required this.scalar,
    required this.valueColumn,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
  });

  @override
  State<_VolumeRow> createState() => _VolumeRowState();
}

class _VolumeRowState extends State<_VolumeRow> {
  /// The value under the pointer during a drag. Unlike a transform or an effect
  /// there is no preview to render — sound is not redrawn — so a tick only holds
  /// the number and the release commits it.
  double? _staged;

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    // From the fold row, never a bridge call here (K-184); the fallback
    // covers a caller that supplied none, which the panel never is.
    final scalar = widget.scalar ?? widget.layer.getVolumeDb();
    final animated = scalar is BridgeScalar_Keyframed;
    final playhead =
        Provider.of<LumitUiState>(context, listen: false).playheadFrame;

    return ValueListenableBuilder<int>(
      valueListenable: playhead,
      builder: (context, frame, _) {
        final value = _staged ??
            (animated
                ? sampledScalar(scalar, timeOfFrame(widget.comp, frame))
                : (scalar as BridgeScalar_Static).field0);
        return Row(
          children: [
            KeyframeControlsFrb(
              scalars: [scalar],
              comp: widget.comp,
              playheadFrame: frame,
              onSeek: widget.onSeek,
              rowKey: 'tl-volume',
              onWrite: (next) {
                widget.layer.setVolumeDb(value: next.single);
                widget.onChanged();
              },
            ),
            const SizedBox(width: 4),
            Expanded(child: Text(l10n.volume, style: t.body)),
            SizedBox(
              width: widget.valueColumn.width,
              // Animated: the change lands in the key under the playhead (or
              // plants one) rather than flattening the curve, and the drag is
              // staged so the whole gesture is one undo step.
              child: animated
                  ? KeyedValueField(
                      fieldKey: const ValueKey('tl-volume-db'),
                      value: value,
                      min: -60,
                      max: 12,
                      decimals: 1,
                      suffix: ' dB',
                      speed: 0.2,
                      onCommit: (v) => _commitAt(scalar, v, frame),
                    )
                  : DragValueField(
                      key: const ValueKey('tl-volume-db'),
                      value: value,
                      // The engine's own range (docs/09 §6): silence to a
                      // +12 dB boost.
                      min: -60,
                      max: 12,
                      decimals: 1,
                      suffix: ' dB',
                      speed: 0.2,
                      onChanged: (v) => _commitAt(scalar, v, frame),
                      onChangeLive: (v) =>
                          setState(() => _staged = v.toDouble()),
                      onChangeEnd: (v) => _commitAt(scalar, v, frame),
                      onDragCancel: () => setState(() => _staged = null),
                    ),
            ),
            SizedBox(width: widget.valueColumn.rightInset),
          ],
        );
      },
    );
  }

  void _commitAt(BridgeScalar scalar, num value, int frame) {
    widget.layer.setVolumeDb(
      value: scalarWithValueAt(scalar, value.toDouble(), widget.comp, frame),
    );
    setState(() => _staged = null);
    widget.onChanged();
  }
}
