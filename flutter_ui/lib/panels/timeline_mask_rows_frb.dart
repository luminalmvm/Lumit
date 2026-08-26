// The Timeline's mask rows: a mask and its values, and the shared opacity row
// every item row in the fold uses.
//
// Split out of timeline_panel_frb.dart.

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';
import '../icons/icons.dart';
import '../l10n/strings.dart';
import '../state/comp_time.dart';
import '../state/timeline_columns.dart';
import '../widgets/controls.dart';
import 'package:lumit_flutter/state/preview_throttle.dart';
import 'timeline_extras_frb.dart';
import 'keyframe_controls_frb.dart';
import 'layer_fold_frb.dart';
import 'timeline_metrics_frb.dart';

/// [m] with one or two fields changed. The engine takes the whole mask, so
/// every edit and every preview here is "the mask, with this changed".
BridgeMask maskWith(
  BridgeMask m, {
  String? name,
  bool? inverted,
  BridgeScalar? opacity,
  BridgeMaskMode? mode,
  BridgeScalar? feather,
  List<BridgeScalar>? vertexFeather,
  BridgeScalar? expansion,
}) =>
    BridgeMask(
      id: m.id,
      name: name ?? m.name,
      vertices: m.vertices,
      closed: m.closed,
      inverted: inverted ?? m.inverted,
      opacity: opacity ?? m.opacity,
      mode: mode ?? m.mode,
      feather: feather ?? m.feather,
      vertexFeather: vertexFeather ?? m.vertexFeather,
      expansion: expansion ?? m.expansion,
      // Where the shape's own keys are is the engine's to say; an edit here
      // never moves them (`set_mask` patches them back).
      pathKeys: m.pathKeys,
    );

/// What a mask mode is called on its dropdown.
String maskModeLabel(BridgeMaskMode mode) => switch (mode) {
      BridgeMaskMode.none => l10n.maskModeNone,
      BridgeMaskMode.add => l10n.maskModeAdd,
      BridgeMaskMode.subtract => l10n.maskModeSubtract,
      BridgeMaskMode.intersect => l10n.maskModeIntersect,
      BridgeMaskMode.lighten => l10n.maskModeLighten,
      BridgeMaskMode.darken => l10n.maskModeDarken,
      BridgeMaskMode.difference => l10n.maskModeDifference,
    };

/// The inline rename shared by the mask row and the shape-item row.
///
/// In plain terms: a shape drawn with the ellipse tool arrives called
/// "Ellipse", which is the right name until it isn't — this is how it becomes
/// "left eye". The name is a label; a double-click (or the row menu's
/// **Rename**) turns it into a field; `Enter` or a click elsewhere keeps what
/// was typed; `Escape` throws it away. An empty name is refused, because a row
/// with no name is worse than a row named after its tool.
///
/// **Why not a single click.** A single tap on these names *selects* the row,
/// and selection is what `Delete` acts on (K-234), so the rename needs a
/// gesture of its own.
///
/// **Why not `onDoubleTap`.** A double-tap recogniser holds every single tap
/// back for the whole double-tap window while the arena waits to see whether a
/// second one is coming — the layer bar found that out beside the razor and
/// counts timestamps instead ([DoubleTap]). The same trade applies here, and
/// worse: selection arriving a third of a second after the click is the thing
/// `Delete` is waiting on. Two timestamps owe the arena nothing.
///
/// The commit is one write through the row's own `_write`, so it is one op and
/// one undo step, exactly as the opacity drag beside it is (K-234, K-240).
mixin _InlineRename<T extends StatefulWidget> on State<T> {
  TextEditingController? _editor;
  final DoubleTap _nameTaps = DoubleTap();

  /// What the row is called now, and how it writes a new name.
  String get renameCurrent;
  void renameCommit(String name);

  /// Open the editor on the current name. Safe to call twice; the second call
  /// leaves the edit in progress alone rather than restarting it.
  void startRename() {
    if (_editor != null) return;
    setState(() => _editor = TextEditingController(text: renameCurrent));
  }

  /// Close the editor, writing what was typed only when [keep].
  void _endRename({required bool keep}) {
    // Both ways out can land here for one edit — submitting and then losing
    // the pointer — and the row can be gone by the time the second arrives.
    if (!mounted || _editor == null) return;
    final text = _editor?.text.trim() ?? '';
    setState(() {
      _editor?.dispose();
      _editor = null;
    });
    if (!keep || text.isEmpty || text == renameCurrent) return;
    renameCommit(text);
  }

  @override
  void dispose() {
    _editor?.dispose();
    super.dispose();
  }

  /// The name cell: the label, or the editor once a rename has started.
  ///
  /// [onTap] still fires on the first tap and at once, so selection is never
  /// held up; the second tap inside the double-tap window opens the editor.
  Widget renameName({
    required String nameKey,
    required String editorKey,
    required TextStyle style,
    VoidCallback? onTap,
  }) {
    final editor = _editor;
    if (editor != null) {
      return Focus(
        // An ancestor of the field, so `Escape` reaches here after the field
        // has had its say: abandon the edit and keep the stored name.
        onKeyEvent: (_, event) {
          if (event is! KeyDownEvent ||
              event.logicalKey != LogicalKeyboardKey.escape) {
            return KeyEventResult.ignored;
          }
          _endRename(keep: false);
          return KeyEventResult.handled;
        },
        child: HouseTextField(
          key: ValueKey<String>(editorKey),
          controller: editor,
          autofocus: true,
          onSubmitted: (_) => _endRename(keep: true),
          // Clicking anywhere else finishes the edit and keeps what was typed,
          // the same as every other inline rename here (K-243).
          onTapOutside: () => _endRename(keep: true),
        ),
      );
    }
    return GestureDetector(
      key: ValueKey<String>(nameKey),
      behavior: HitTestBehavior.opaque,
      onTap: () {
        onTap?.call();
        if (_nameTaps.tap()) startRename();
      },
      child: Text(renameCurrent, style: style, overflow: TextOverflow.ellipsis),
    );
  }
}

/// One mask's row in the fold-out (K-222): its name, its mode, its invert
/// switch and its opacity. Its feather and its expansion are rows of their own
/// underneath, because the value column holds one field.
///
/// Read from the model, written through the layer's own handle — the same shape
/// as every other row here. Deleting a mask is on its right-click menu, and on
/// the Delete key once the row is selected; a button per mask on every row is a
/// row of ways to lose work by mistake.
///
/// The row is selectable like any other property (K-234): tapping its name
/// calls [onLabelTap], the outline highlights it, and Delete acts on it.
class MaskRow extends StatefulWidget {
  final LayerReference layer;
  final BridgeMask mask;
  final ValueColumn valueColumn;

  final VoidCallback onChanged;
  final VoidCallback? onLabelTap;

  /// The composition, for the live preview a drag shows (K-240).
  final CompositionReference comp;

  const MaskRow({super.key, 
    required this.layer,
    required this.mask,
    required this.valueColumn,
    required this.onChanged,
    required this.comp,
    this.onLabelTap,
  });

  @override
  State<MaskRow> createState() => _MaskRowState();
}

class _MaskRowState extends State<MaskRow> with _InlineRename<MaskRow> {
  @override
  String get renameCurrent => widget.mask.name;

  @override
  void renameCommit(String name) => _write(name: name);

  /// Write the mask back with one field changed. The engine takes the whole
  /// mask, so this is the only shape an edit has.
  void _write({
    String? name,
    bool? inverted,
    BridgeMaskMode? mode,
    List<BridgeScalar>? vertexFeather,
  }) {
    try {
      widget.layer.setMask(
        mask: maskWith(widget.mask,
            name: name,
            inverted: inverted,
            mode: mode,
            vertexFeather: vertexFeather),
      );
      widget.onChanged();
    } catch (_) {
      // The mask or its layer went away between the draw and the click.
    }
  }

  @override
  Widget build(BuildContext context) {
    final mask = widget.mask;
    final valueColumn = widget.valueColumn;
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onSecondaryTapUp: (details) => _menu(context, details.globalPosition),
      child: Row(
        children: [
          lumitIcon(LumitIcon.rectangle,
              size: iconSize, color: t.textSecondary),
          const SizedBox(width: 4),
          // The name is the row's handle, exactly as it is on a transform row:
          // tapping it selects the mask, and Delete then acts on it. A
          // double-click renames it in place, and so does the row menu.
          Expanded(
            child: renameName(
              nameKey: 'tl-mask-name-${mask.id}',
              editorKey: 'tl-mask-rename-${mask.id}',
              style: t.body,
              onTap: widget.onLabelTap,
            ),
          ),
          // **Both of the mask's own switches live in the value column**, where
          // every other row's control sits, rather than floating beside the
          // name: the invert mark and the mode picker are what the mask *is*,
          // and a control that sits in no column reads as belonging to nothing.
          SizedBox(
            width: valueColumn.width,
            child: Row(
              children: [
                LumitTooltip(
                  message: l10n.tipInvert,
                  child: HouseButton(
                    key: ValueKey<String>('tl-mask-invert-${mask.id}'),
                    small: true,
                    frameless: true,
                    onPressed: () => _write(inverted: !mask.inverted),
                    child: Text(
                      l10n.maskInvertMark,
                      style: t.small.copyWith(
                          color: mask.inverted ? t.accent : t.textMuted),
                    ),
                  ),
                ),
                const SizedBox(width: 6),
                // The rest of the cell, so a long mode name ellipsises rather
                // than pushing the row wider than its column — the same rule
                // the blend picker follows.
                Expanded(
                  child: LumitTooltip(
                    message: l10n.tipMaskMode,
                    child: BareDropdown<BridgeMaskMode>(
                      key: ValueKey<String>('tl-mask-mode-${mask.id}'),
                      value: mask.mode,
                      options: BridgeMaskMode.values,
                      label: maskModeLabel,
                      onChanged: (m) => _write(mode: m),
                    ),
                  ),
                ),
              ],
            ),
          ),
          SizedBox(width: valueColumn.rightInset),
        ],
      ),
    );
  }

  void _menu(BuildContext context, Offset at) {
    showMenuAt<void>(
      context: context,
      position: at,
      width: 160,
      rows: (close) => [
        MenuRow(
          key: ValueKey<String>('tl-mask-rename-menu-${widget.mask.id}'),
          onPressed: () {
            close(null);
            startRename();
          },
          // The same bare "Rename" the Project panel's row menu offers.
          child: Text(l10n.rename),
        ),
        // **Where a varying feather is switched on** (K-545). Turning it on
        // gives every point the width the mask already had, so the picture
        // does not move until a point is actually dragged; turning it off
        // drops the points and the one width stands again.
        MenuRow(
          key: ValueKey<String>('tl-mask-vary-feather-${widget.mask.id}'),
          onPressed: () {
            close(null);
            final on = widget.mask.vertexFeather.isNotEmpty;
            _write(
              vertexFeather: on
                  ? const []
                  : List<BridgeScalar>.filled(
                      widget.mask.vertices.length, widget.mask.feather),
            );
          },
          child: Text(widget.mask.vertexFeather.isEmpty
              ? l10n.maskFeatherPerPoint
              : l10n.maskFeatherOneWidth),
        ),
        MenuRow(
          key: ValueKey<String>('tl-mask-delete-${widget.mask.id}'),
          onPressed: () {
            close(null);
            try {
              widget.layer.deleteMask(id: widget.mask.id);
              widget.onChanged();
            } catch (_) {}
          },
          child: Text(l10n.deleteMask),
        ),
      ],
    );
  }
}

/// One of a mask's values on a row under it (K-222, K-340): its shape, its
/// opacity, its feather — one width or one point's own (K-545) — or its
/// expansion.
///
/// **Every one of them animates, and animates the way everything else does.**
/// The row carries the same stopwatch and ◄ ◆ ► the transform and effect rows
/// carry, reads its value at the playhead, and writes an edit into the key
/// sitting there — so a mask is keyed with the same gesture as a position.
///
/// The **shape** is the exception in one respect only: a path has no number to
/// put in a field, so its row is a name, a stopwatch and its diamonds, and the
/// shape itself is edited where it is drawn (K-339).
///
/// The drag is staged and previewed exactly as it always was, so the whole
/// gesture is one op and one undo step (K-234, K-240).
///
/// The row has no label tap: the mask itself is what Delete acts on, and a
/// selectable value row under it would give Delete a path it cannot resolve to
/// a mask.
class MaskValueRow extends StatefulWidget {
  final LayerReference layer;
  final CompositionReference comp;
  final BridgeMask mask;
  final MaskValue value;

  /// Which point this row's width belongs to, for a per-point feather row;
  /// `-1` on every other row (K-545).
  final int vertex;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  /// Clicking the name selects the property and its keys (K-500 §2.1).
  final VoidCallback? onLabelTap;

  const MaskValueRow({super.key, 
    required this.layer,
    required this.comp,
    required this.mask,
    required this.value,
    required this.valueColumn,
    required this.playheadFrame,
    required this.onSeek,
    required this.onChanged,
    this.onLabelTap,
    this.vertex = -1,
  });

  @override
  State<MaskValueRow> createState() => _MaskValueRowState();
}

class _MaskValueRowState extends State<MaskValueRow> {
  double? _staged;
  final PreviewThrottle _throttle = PreviewThrottle();

  bool get _isPath => widget.value == MaskValue.path;

  /// This row's animation. The path has none of its own — its keys are whole
  /// shapes, not numbers — so [maskScalarOf] answers a still zero for it.
  BridgeScalar get _scalar =>
      maskScalarOf(widget.mask, widget.value, widget.vertex);

  /// What a drag on this row may ask for. Feather is a width, so it has no
  /// negative side; expansion grows one way and shrinks the other; opacity is
  /// a percentage.
  (double, double) get _range => switch (widget.value) {
        MaskValue.opacity => (0, 100),
        MaskValue.feather || MaskValue.vertexFeather => (0, 1000),
        _ => (-1000, 1000),
      };

  int get _decimals => widget.value == MaskValue.opacity ? 0 : 1;

  String get _suffix => widget.value == MaskValue.opacity ? '%' : ' px';

  @override
  void dispose() {
    _throttle.cancel();
    super.dispose();
  }

  /// Show the value the drag is passing through without writing it (K-240).
  void _preview(BridgeScalar v) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    _throttle.request(() {
      try {
        widget.comp.renderFrameWithMaskPreview(
          frame: BigInt.from(ui.playheadFrame.value),
          scale: ui.viewerScale,
          layer: widget.layer,
          masks: [
            for (final m in widget.layer.getMasks())
              if (m.id == widget.mask.id)
                maskWithScalar(m, widget.value, v, widget.vertex)
              else
                m,
          ],
        );
      } catch (_) {
        // A preview is a courtesy; the drag carries on without it.
      }
    });
  }

  void _write(BridgeScalar v) {
    setState(() => _staged = null);
    try {
      widget.layer.setMask(
          mask: maskWithScalar(widget.mask, widget.value, v, widget.vertex));
      widget.onChanged();
    } catch (_) {
      // The mask or its layer went away mid-drag.
    }
  }

  /// A still value: the number typed or dragged becomes the value.
  void _commitStatic(num v) => _write(BridgeScalar.static_(v.toDouble()));

  /// An animated one: the edit lands on the key under the playhead, or plants
  /// one there — never flattening the curve (docs/07 §4.3).
  void _commitKeyed(double v) =>
      _write(scalarWithValueAt(_scalar, v, widget.comp, widget.playheadFrame));

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return Row(
      children: [
        if (_isPath)
          PathKeyframesFrb(
            keys: widget.mask.pathKeys,
            rowKey: 'tl-mask-path-${widget.mask.id}',
            onToggleKey: (time) =>
                widget.layer.toggleMaskPathKey(id: widget.mask.id, time: time),
            onClear: (time) =>
                widget.layer.clearMaskPathKeys(id: widget.mask.id, time: time),
            comp: widget.comp,
            playheadFrame: widget.playheadFrame,
            onSeek: widget.onSeek,
            onChanged: widget.onChanged,
          )
        else
          KeyframeControlsFrb(
            scalars: [_scalar],
            onWrite: (s) => _write(s.first),
            comp: widget.comp,
            playheadFrame: widget.playheadFrame,
            onSeek: widget.onSeek,
            rowKey: _rowKey,
          ),
        const SizedBox(width: 4),
        Expanded(
          child: GestureDetector(
            behavior: HitTestBehavior.opaque,
            onTap: widget.onLabelTap,
            child: Row(children: [
              Flexible(
                child: Text(maskValueLabel(widget.value, widget.vertex),
                    style: t.body, overflow: TextOverflow.ellipsis),
              ),
            ]),
          ),
        ),
        // Left of the value column, exactly where an effect parameter's field
        // sits, so every number down an open layer forms one column.
        SizedBox(
          width: widget.valueColumn.width,
          child: _isPath
              ? const SizedBox.shrink()
              : Align(
                  alignment: Alignment.centerLeft,
                  child: SizedBox(width: 72, child: _field()),
                ),
        ),
        SizedBox(width: widget.valueColumn.rightInset),
      ],
    );
  }

  /// This row's key, which per-point feather rows must not share: they are
  /// several rows of the same value on the same mask (K-545).
  String get _rowKey => 'tl-mask-${widget.value.name}-${widget.mask.id}'
      '${widget.vertex < 0 ? '' : '-${widget.vertex}'}';

  Widget _field() {
    final (min, max) = _range;
    final key = ValueKey<String>(_rowKey);
    final scalar = _scalar;
    if (scalar is! BridgeScalar_Keyframed) {
      final stored =
          _staged ?? (scalar is BridgeScalar_Static ? scalar.field0 : 0.0);
      return DragValueField(
        key: key,
        value: stored,
        min: min,
        max: max,
        decimals: _decimals,
        suffix: _suffix,
        onChanged: _commitStatic,
        onChangeLive: (v) {
          setState(() => _staged = v.toDouble());
          _preview(BridgeScalar.static_(v.toDouble()));
        },
        onChangeEnd: _commitStatic,
        onDragCancel: () {
          setState(() => _staged = null);
          // Put the document's own value back on screen.
          _preview(scalar);
        },
      );
    }
    // Animated: the field shows what the curve reads at the playhead, and an
    // edit writes the key there. No live preview mid-drag — staging a keyed
    // value through the static preview would lie about the curve.
    return KeyedValueField(
      fieldKey: key,
      value:
          sampledScalar(scalar, timeOfFrame(widget.comp, widget.playheadFrame)),
      min: min,
      max: max,
      decimals: _decimals,
      suffix: _suffix,
      onCommit: _commitKeyed,
    );
  }
}

/// One named, deletable item with an opacity of its own — a piece of a shape
/// layer's art (K-237) or a paint stroke (K-227). The two rows were twins:
/// an icon, the name, the staged-and-previewed opacity drag, and the
/// right-click menu that deletes it. What differs — how a preview is asked
/// for, how an edit is written, whether the name renames — comes in as
/// callbacks from the two thin rows below.
///
/// The drag is staged and previewed like every other dragged value here: the
/// tick shows live and the release commits once, so a gesture is one op and
/// one undo step (K-238, K-239).
class ItemOpacityRow extends StatefulWidget {
  final LumitIcon icon;
  final String name;

  /// The widget keys' stem: `<keyPrefix>-name-<id>` and so on, kept exactly
  /// as the two original rows spelt them.
  final String keyPrefix;
  final String id;
  final double opacity;
  final ValueColumn valueColumn;

  /// Render the picture with [opacity] in place of the stored one; called
  /// from inside the row's own throttle.
  final void Function(double opacity) onPreview;

  /// Commit [opacity] as one op.
  final void Function(double opacity) onCommit;

  /// Write a new name, or null when this kind's name is not renamed here —
  /// which also drops the menu's Rename row.
  final void Function(String name)? onRename;
  final VoidCallback onDelete;
  final String deleteLabel;

  /// A control of the item's own, drawn between the name and the value column
  /// — a paint stroke's blend mode (K-550). Null for a shape item, which has
  /// no such choice.
  final Widget? extra;

  const ItemOpacityRow({super.key, 
    required this.icon,
    required this.name,
    required this.keyPrefix,
    required this.id,
    required this.opacity,
    required this.valueColumn,
    required this.onPreview,
    required this.onCommit,
    this.onRename,
    required this.onDelete,
    required this.deleteLabel,
    this.extra,
  });

  @override
  State<ItemOpacityRow> createState() => _ItemOpacityRowState();
}

class _ItemOpacityRowState extends State<ItemOpacityRow>
    with _InlineRename<ItemOpacityRow> {
  /// The opacity a drag is part way through, or null when nothing is
  /// dragging. Without it the field committed on every tick, so one drag was
  /// a stack of ops and `Ctrl+Z` backed out a hair (K-238, K-239).
  double? _staged;

  final PreviewThrottle _throttle = PreviewThrottle();

  @override
  String get renameCurrent => widget.name;

  @override
  void renameCommit(String name) => widget.onRename?.call(name);

  @override
  void dispose() {
    _throttle.cancel();
    super.dispose();
  }

  void _preview(double opacity) =>
      _throttle.request(() => widget.onPreview(opacity));

  void _commitOpacity(num v) {
    setState(() => _staged = null);
    widget.onCommit(v.toDouble());
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onSecondaryTapUp: (details) => _menu(context, details.globalPosition),
      child: Row(
        children: [
          lumitIcon(widget.icon, size: iconSize, color: t.textSecondary),
          const SizedBox(width: 4),
          // Named after the tool that drew it — and, where the kind supports
          // it, renamed here: a double-click on the name, or the row menu.
          Expanded(
            child: widget.onRename == null
                ? Text(widget.name,
                    style: t.body, overflow: TextOverflow.ellipsis)
                : renameName(
                    nameKey: '${widget.keyPrefix}-name-${widget.id}',
                    editorKey: '${widget.keyPrefix}-rename-${widget.id}',
                    style: t.body,
                  ),
          ),
          if (widget.extra != null) ...[
            widget.extra!,
            const SizedBox(width: 6),
          ],
          SizedBox(
            width: widget.valueColumn.width,
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                SizedBox(
                  width: 56,
                  // Staged and previewed, like every other dragged value
                  // here: the drag shows live and commits once on release,
                  // so it is one op and one undo step.
                  child: DragValueField(
                    key: ValueKey<String>(
                        '${widget.keyPrefix}-opacity-${widget.id}'),
                    value: _staged ?? widget.opacity,
                    min: 0,
                    max: 100,
                    suffix: '%',
                    onChanged: _commitOpacity,
                    onChangeLive: (v) {
                      setState(() => _staged = v.toDouble());
                      _preview(v.toDouble());
                    },
                    onChangeEnd: _commitOpacity,
                    onDragCancel: () {
                      setState(() => _staged = null);
                      // The picture is showing a value nobody committed; put
                      // the document's own back on screen.
                      _preview(widget.opacity);
                    },
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }

  void _menu(BuildContext context, Offset at) {
    showMenuAt<void>(
      context: context,
      position: at,
      width: 160,
      rows: (close) => [
        if (widget.onRename != null)
          MenuRow(
            key: ValueKey<String>(
                '${widget.keyPrefix}-rename-menu-${widget.id}'),
            onPressed: () {
              close(null);
              startRename();
            },
            child: Text(l10n.rename),
          ),
        MenuRow(
          key: ValueKey<String>('${widget.keyPrefix}-delete-${widget.id}'),
          onPressed: () {
            close(null);
            widget.onDelete();
          },
          child: Text(widget.deleteLabel),
        ),
      ],
    );
  }
}
