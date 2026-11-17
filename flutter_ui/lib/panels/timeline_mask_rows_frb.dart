// The Timeline's mask rows: a mask and its values, and the shared opacity row
// every item row in the fold uses.
//
// Split out of timeline_panel_frb.dart.

import 'dart:convert';

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
import '../shell/menu_bar_frb.dart' show MenuEntry;
import '../shell/menu_layer_frb.dart' show maskRows, pickedMaskSet;
import '../state/clipboard.dart' show ClipboardKind;
import '../state/template_controls.dart' show ControlRow;
import '../state/timeline_columns.dart';
import '../widgets/controls.dart';
import 'package:lumit_flutter/state/preview_throttle.dart';
import 'control_rows.dart' show ControlMark, maskControlRow;
import 'timeline_extras_frb.dart';
import 'keyframe_controls_frb.dart';
import 'param_row_menu.dart';
import 'layer_fold_frb.dart';
import 'timeline_layer_menu_frb.dart' show showMenuEntriesAt;

/// [m] with one or two fields changed. The engine takes the whole mask, so
/// every edit and every preview here is "the mask, with this changed".
BridgeMask maskWith(
  BridgeMask m, {
  String? name,
  List<BridgeVertex>? vertices,
  bool? closed,
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
      vertices: vertices ?? m.vertices,
      closed: closed ?? m.closed,
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
/// and selection is what `Delete` acts on, so the rename needs a
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
/// one undo step, exactly as the opacity drag beside it is.
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
          // the same as every other inline rename here.
          onTapOutside: () => _endRename(keep: true),
        ),
      );
    }
    return HoverMark(
      builder: (context, lit) => GestureDetector(
        key: ValueKey<String>(nameKey),
        behavior: HitTestBehavior.opaque,
        onTap: () {
          onTap?.call();
          if (_nameTaps.tap()) startRename();
        },
        child: Text(renameCurrent,
            style: style.copyWith(
                color: Color.lerp(style.color,
                    ThemeScope.of(context).theme.textPrimary, lit)),
            overflow: TextOverflow.ellipsis),
      ),
    );
  }
}

/// One mask's row in the fold-out: its name, its mode, its invert
/// switch and its opacity. Its feather and its expansion are rows of their own
/// underneath, because the value column holds one field.
///
/// Read from the model, written through the layer's own handle — the same shape
/// as every other row here. Deleting a mask is on its right-click menu, and on
/// the Delete key once the row is selected; a button per mask on every row is a
/// row of ways to lose work by mistake.
///
/// The row is selectable like any other property: tapping its name
/// calls [onLabelTap], the outline highlights it, and Delete acts on it.
class MaskRow extends StatefulWidget {
  final LayerReference layer;
  final BridgeMask mask;
  final ValueColumn valueColumn;

  final VoidCallback onChanged;
  final VoidCallback? onLabelTap;

  /// The composition, for the live preview a drag shows.
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
                    child: glyph.LumitIcon(LumitIcons.invert,
                        size: iconSize,
                        colour: mask.inverted ? t.accent : t.textMuted),
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

  /// The mask's menu: its name, the Layer menu's own Mask rows on this mask,
  /// then the clipboard and Delete, which act on every picked mask. A mask
  /// outside the pick is picked first, so the rows know which one.
  void _menu(BuildContext context, Offset at) {
    final app = Provider.of<LumitState>(context, listen: false);
    final ui = Provider.of<LumitUiState>(context, listen: false);
    final mine = pickedMasks(ui).any((p) => p.$2.contains(widget.mask.id));
    if (!mine) widget.onLabelTap?.call();
    final picked = pickedMaskSet(ui);
    final perPoint = picked.isNotEmpty &&
        picked.every((p) => p.mask.vertexFeather.isNotEmpty);
    showMenuEntriesAt(context, at, [
      MenuEntry(l10n.rename, startRename, action: 'layer.rename'),
      MenuEntry.divider(),
      ...maskRows(context, app, ui),
      // Where a varying feather is switched on, on every picked mask.
      // Turning it on gives every point the width the mask already had, so
      // the picture does not move until a point is dragged; off drops the
      // points and the one width stands again.
      MenuEntry(
          perPoint ? l10n.maskFeatherOneWidth : l10n.maskFeatherPerPoint, () {
        asOneUndoStep(app.project, () {
          for (final (:layer, :mask) in picked) {
            if (mask.vertexFeather.isNotEmpty != perPoint) continue;
            try {
              layer.setMask(
                mask: maskWith(mask,
                    vertexFeather: perPoint
                        ? const []
                        : List<BridgeScalar>.filled(
                            mask.vertices.length, mask.feather)),
              );
            } catch (_) {}
          }
        });
        widget.onChanged();
      }),
      MenuEntry.divider(),
      MenuEntry(l10n.menuCut, () {
        if (copyPickedMasks(ui)) deletePickedMasks(app, ui);
      }, action: 'edit.cut'),
      MenuEntry(l10n.menuCopy, () => copyPickedMasks(ui), action: 'edit.copy'),
      MenuEntry(
          l10n.menuPaste,
          ui.clipboard.kind == ClipboardKind.masks
              ? () => pasteMasksOnto(app, ui, [widget.layer])
              : null,
          action: 'edit.paste'),
      MenuEntry(l10n.menuDuplicate, () => duplicatePickedMasks(app, ui),
          action: 'layer.duplicate'),
      MenuEntry.divider(),
      MenuEntry(l10n.deleteMask, () => deletePickedMasks(app, ui),
          action: 'edit.delete.selection'),
    ]);
  }
}

/// What Copy path on a mask's Path row kept, for Paste path on another.
({List<BridgeVertex> vertices, bool closed})? _copiedMaskPath;

/// The masks picked in the Timeline, by the layer each is on, in stack order.
/// A mask's own row picks it; a row under it picks a value, not the mask.
List<(LayerReference, List<UuidValue>)> pickedMasks(LumitUiState ui) {
  final wanted = <String, Set<String>>{};
  for (final path in ui.selectedProperties.value) {
    final layerId = layerIdOfPath(path);
    if (layerId == null) continue;
    final prefix = '${masksPath(layerId)}/';
    final rest = path.startsWith(prefix) ? path.substring(prefix.length) : '';
    if (rest.isEmpty || rest.contains('/')) continue;
    (wanted[layerId] ??= {}).add(rest);
  }
  return [
    for (final entry in ui.model.layers)
      if (wanted[entry.layer.internallayerId.toString()] case final ids?)
        (
          entry.layer,
          [
            for (final m in entry.info.masks)
              if (ids.contains(m.id.toString())) m.id,
          ]
        ),
  ];
}

/// Put the picked masks on the clipboard, from every layer they are on. False
/// with none picked.
bool copyPickedMasks(LumitUiState ui) {
  final masks = <Object?>[];
  for (final (layer, ids) in pickedMasks(ui)) {
    try {
      final copied = jsonDecode(layer.copyMasks(masks: ids)) as Map;
      masks.addAll(copied['masks'] as List);
    } catch (_) {
      // Gone between the pick and the press.
    }
  }
  if (masks.isEmpty) return false;
  ui.copyMasksToClipboard(
      jsonEncode({'format': 1, 'kind': 'masks', 'masks': masks}));
  return true;
}

/// Take the picked masks away, as one undo step.
bool deletePickedMasks(LumitState app, LumitUiState ui) {
  final picked = pickedMasks(ui);
  if (picked.isEmpty) return false;
  asOneUndoStep(app.project, () {
    for (final (layer, ids) in picked) {
      for (final id in ids) {
        try {
          layer.deleteMask(id: id);
        } catch (_) {}
      }
    }
  });
  app.notifyDocumentChanged();
  ui.model.refresh();
  return true;
}

/// A copy of each picked mask, right after it, as one undo step.
bool duplicatePickedMasks(LumitState app, LumitUiState ui) {
  final picked = pickedMasks(ui);
  if (picked.isEmpty) return false;
  asOneUndoStep(app.project, () {
    for (final (layer, ids) in picked) {
      try {
        layer.duplicateMasks(masks: ids);
      } catch (_) {}
    }
  });
  app.notifyDocumentChanged();
  ui.model.refresh();
  return true;
}

/// The clipboard's masks onto each of [layers], as one undo step.
bool pasteMasksOnto(
    LumitState app, LumitUiState ui, List<LayerReference> layers) {
  final text = ui.clipboard.text;
  if (ui.clipboard.kind != ClipboardKind.masks || text == null) return false;
  if (layers.isEmpty) return false;
  asOneUndoStep(app.project, () {
    for (final layer in layers) {
      try {
        layer.pasteMasks(text: text);
      } catch (_) {
        // A layer that takes no masks, or one that has gone.
      }
    }
  });
  app.notifyDocumentChanged();
  ui.model.refresh();
  return true;
}

/// One of a mask's values on a row under it: its shape, its
/// opacity, its feather — one width or one point's own — or its
/// expansion.
///
/// **Every one of them animates, and animates the way everything else does.**
/// The row carries the same stopwatch and ◄ ◆ ► the transform and effect rows
/// carry, reads its value at the playhead, and writes an edit into the key
/// sitting there — so a mask is keyed with the same gesture as a position.
///
/// The **shape** is the exception in one respect only: a path has no number to
/// put in a field, so its row is a name, a stopwatch and its diamonds, and the
/// shape itself is edited where it is drawn.
///
/// The drag is staged and previewed exactly as it always was, so the whole
/// gesture is one op and one undo step.
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
  /// `-1` on every other row.
  final int vertex;
  final ValueColumn valueColumn;
  final int playheadFrame;
  final ValueChanged<int> onSeek;
  final VoidCallback onChanged;

  /// Clicking the name selects the property and its keys.
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

  /// Show the value the drag is passing through without writing it.
  void _preview(BridgeScalar v) {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    _throttle.request(() {
      try {
        widget.comp.renderFrameWithMaskPreview(
          frame: ui.playheadFrame.value,
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

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    final keys = KeyframeControlsFrb(
      scalars: [_scalar],
      onWrite: (s) => _write(s.first),
      comp: widget.comp,
      onSeek: widget.onSeek,
      rowKey: _rowKey,
      navigatorApart: true,
    );
    final pathKeys = !_isPath
        ? null
        : PathKeyframesFrb(
            keys: widget.mask.pathKeys,
            rowKey: 'tl-mask-path-${widget.mask.id}',
            onToggleKey: (time) =>
                widget.layer.toggleMaskPathKey(id: widget.mask.id, time: time),
            onClear: (time) =>
                widget.layer.clearMaskPathKeys(id: widget.mask.id, time: time),
            comp: widget.comp,
            onSeek: widget.onSeek,
            onChanged: widget.onChanged,
            navigatorApart: true,
          );
    final RowKeys lead = pathKeys ?? keys;
    // Opacity, Feather and Expansion can be controls. The path cannot.
    final control = maskControlRow(
        widget.layer.internallayerId, widget.mask.id, widget.value);
    final row = Row(
      children: [
        pathKeys ?? keys,
        const SizedBox(width: 4),
        Expanded(
          child: HoverMark(
            builder: (context, lit) => GestureDetector(
              behavior: HitTestBehavior.opaque,
              onTap: widget.onLabelTap,
              child: Row(children: [
                Flexible(
                  child: Text(
                      t.propertyCase(
                          maskValueLabel(widget.value, widget.vertex)),
                      style: t.body.copyWith(
                          color: Color.lerp(t.body.color, t.textPrimary, lit)),
                      overflow: TextOverflow.ellipsis),
                ),
                ControlMark(control),
              ]),
            ),
          ),
        ),
        if (lead.trailingNavigator case final nav?) nav,
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
    // A shape is no number to type, so its menu is the shape to copy onto
    // another mask, its keys and its curve.
    if (pathKeys != null) {
      return ParamRowMenu(
        menu: ParamMenu(
          own: (close) => [
            MenuRow(
              key: const ValueKey('param-menu-copy-path'),
              onPressed: () {
                close();
                _copyPath();
              },
              child: MenuLabel(l10n.maskCopyPath),
            ),
            MenuRow(
              key: const ValueKey('param-menu-paste-path'),
              enabled: _copiedMaskPath != null,
              onPressed: () {
                close();
                _pastePath();
              },
              child: MenuLabel(l10n.maskPastePath,
                  enabled: _copiedMaskPath != null),
            ),
          ],
          keys: pathKeys,
          graphPath: ParamRowPath.of(context),
        ),
        child: row,
      );
    }
    return ParamRowMenu(
      menu: scalarRowMenu(context, keys,
          defaults: [widget.value == MaskValue.opacity ? 100 : 0],
          control:
              control == null ? null : ControlRow(widget.comp, control)),
      child: row,
    );
  }

  /// Keep the shape the mask shows at the playhead: its keys' shape there when
  /// it is keyed, else its one shape.
  void _copyPath() {
    final ui = Provider.of<LumitUiState>(context, listen: false);
    var vertices = widget.mask.vertices;
    if (widget.mask.pathKeys.isNotEmpty) {
      try {
        final moving = widget.comp
            .animatedMaskPathsAt(frame: ui.playheadFrame.value)
            .where((p) => p.mask == widget.mask.id)
            .firstOrNull;
        if (moving != null) vertices = moving.vertices;
      } catch (_) {}
    }
    _copiedMaskPath = (vertices: vertices, closed: widget.mask.closed);
  }

  /// Give this mask the copied shape, and every picked mask with it when it is
  /// one of them, as one undo step: a key at the playhead where a shape is
  /// keyed, as a drag of its points would write, else its one shape.
  void _pastePath() {
    final copied = _copiedMaskPath;
    if (copied == null) return;
    final app = Provider.of<LumitState>(context, listen: false);
    final ui = Provider.of<LumitUiState>(context, listen: false);
    // Picked by its own row or by its Path row.
    final paths = ui.selectedProperties.value;
    final picked = [
      for (final entry in ui.model.layers)
        for (final mask in entry.info.masks)
          if ('${masksPath(entry.layer.internallayerId.toString())}/${mask.id}'
                  case final own
              when paths.contains(own) || paths.contains('$own/path'))
            (layer: entry.layer, mask: mask),
    ];
    final targets = picked.any((p) => p.mask.id == widget.mask.id)
        ? picked
        : [(layer: widget.layer, mask: widget.mask)];
    final at = widget.comp.timeOfFrame(frame: ui.playheadFrame.value);
    asOneUndoStep(app.project, () {
      for (final (:layer, :mask) in targets) {
        try {
          layer.setMask(
            mask: maskWith(mask,
                vertices: copied.vertices, closed: copied.closed),
            at: mask.pathKeys.isEmpty ? null : at,
          );
        } catch (_) {
          // The mask or its layer went away under the open menu.
        }
      }
    });
    widget.onChanged();
  }

  /// This row's key, which per-point feather rows must not share: they are
  /// several rows of the same value on the same mask.
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
    return PlayheadKeyedField(
      fieldKey: key,
      scalar: scalar,
      comp: widget.comp,
      min: min,
      max: max,
      decimals: _decimals,
      suffix: _suffix,
      onWrite: _write,
    );
  }
}

/// One named, deletable item with an opacity of its own — a piece of a shape
/// layer's art or a paint stroke. The two rows were twins:
/// an icon, the name, the staged-and-previewed opacity drag, and the
/// right-click menu that deletes it. What differs — how a preview is asked
/// for, how an edit is written, whether the name renames — comes in as
/// callbacks from the two thin rows below.
///
/// The drag is staged and previewed like every other dragged value here: the
/// tick shows live and the release commits once, so a gesture is one op and
/// one undo step.
class ItemOpacityRow extends StatefulWidget {
  final LumitIcon icon;
  final String name;

  /// The widget keys' stem: `<keyPrefix>-name-<id>` and so on, kept exactly
  /// as the two original rows spelt them.
  final String keyPrefix;
  final String id;

  /// The item's opacity, or **null** for a kind that has none — a puppet pin,
  /// which is a place rather than a mark and has nothing to be see through. A
  /// null leaves the value column empty and drops the preview and the commit
  /// with it; everything else about the row — the icon, the inline rename, the
  /// right-click menu, the Delete row — is the same.
  final double? opacity;
  final ValueColumn valueColumn;

  /// Render the picture with [opacity] in place of the stored one; called
  /// from inside the row's own throttle. Unused when [opacity] is null.
  final void Function(double opacity)? onPreview;

  /// Commit [opacity] as one op. Unused when [opacity] is null.
  final void Function(double opacity)? onCommit;

  /// Write a new name, or null when this kind's name is not renamed here —
  /// which also drops the menu's Rename row.
  final void Function(String name)? onRename;
  final VoidCallback onDelete;
  final String deleteLabel;

  /// A control of the item's own, drawn between the name and the value column
  /// — a paint stroke's blend mode. Null for a shape item, which has
  /// no such choice.
  final Widget? extra;

  /// Rows of the item's own for its right-click menu, between Rename and
  /// Delete — a shape item's Add and Remove. Handed the menu's close.
  final List<Widget> Function(VoidCallback close)? menuRows;

  /// The rows an **Add** button beside the name opens, After Effects' "Add ▸"
  /// on a shape's own entry rather than only on Contents. Null draws none.
  final List<Widget> Function(VoidCallback close)? addRows;

  /// Draws a **Remove** button beside the name, which runs [onDelete]: a
  /// shape modifier's. False draws none.
  final bool removeButton;

  /// A twirl before the icon, for an item with rows of its own under it — a
  /// shape item. Null draws none.
  final bool open;
  final VoidCallback? onToggle;

  /// Asks for a rename by id, the way `Enter` does a layer's. Null for a kind
  /// that is only renamed by hand.
  final ValueNotifier<String?>? renameRequest;

  /// A tap on the name, which picks the item the way a mask's name picks the
  /// mask.
  final VoidCallback? onLabelTap;

  const ItemOpacityRow({super.key,
    required this.icon,
    required this.name,
    required this.keyPrefix,
    required this.id,
    this.opacity,
    required this.valueColumn,
    this.onPreview,
    this.onCommit,
    this.onRename,
    required this.onDelete,
    required this.deleteLabel,
    this.extra,
    this.menuRows,
    this.addRows,
    this.removeButton = false,
    this.open = false,
    this.onToggle,
    this.renameRequest,
    this.onLabelTap,
  });

  @override
  State<ItemOpacityRow> createState() => _ItemOpacityRowState();
}

class _ItemOpacityRowState extends State<ItemOpacityRow>
    with _InlineRename<ItemOpacityRow> {
  /// The opacity a drag is part way through, or null when nothing is
  /// dragging. Without it the field committed on every tick, so one drag was
  /// a stack of ops and `Ctrl+Z` backed out a hair.
  double? _staged;

  final PreviewThrottle _throttle = PreviewThrottle();

  @override
  String get renameCurrent => widget.name;

  @override
  void renameCommit(String name) => widget.onRename?.call(name);

  @override
  void initState() {
    super.initState();
    widget.renameRequest?.addListener(_maybeRename);
  }

  @override
  void didUpdateWidget(ItemOpacityRow old) {
    super.didUpdateWidget(old);
    if (old.renameRequest == widget.renameRequest) return;
    old.renameRequest?.removeListener(_maybeRename);
    widget.renameRequest?.addListener(_maybeRename);
  }

  void _maybeRename() {
    final request = widget.renameRequest;
    if (request == null || request.value != widget.id) return;
    request.value = null;
    startRename();
  }

  @override
  void dispose() {
    widget.renameRequest?.removeListener(_maybeRename);
    _throttle.cancel();
    super.dispose();
  }

  void _preview(double opacity) =>
      _throttle.request(() => widget.onPreview?.call(opacity));

  void _commitOpacity(num v) {
    setState(() => _staged = null);
    widget.onCommit?.call(v.toDouble());
  }

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return GestureDetector(
      behavior: HitTestBehavior.opaque,
      onSecondaryTapUp: (details) => _menu(context, details.globalPosition),
      child: Row(
        children: [
          if (widget.onToggle case final toggle?) ...[
            // The same twirl a heading carries, and as wide to aim at.
            GestureDetector(
              key: ValueKey<String>('${widget.keyPrefix}-twirl-${widget.id}'),
              behavior: HitTestBehavior.opaque,
              onTap: toggle,
              child: SizedBox(
                width: iconSize + 6,
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
            const SizedBox(width: 4),
          ],
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
                    onTap: widget.onLabelTap,
                  ),
          ),
          if (widget.extra != null) ...[
            widget.extra!,
            const SizedBox(width: 6),
          ],
          if (widget.addRows case final rows?) ...[
            RowButton(
              key: ValueKey<String>('${widget.keyPrefix}-add-button-${widget.id}'),
              label: l10n.shapeOpAdd,
              arrow: true,
              onTap: (at) => showMenuAt<void>(
                context: context,
                position: at,
                width: 150,
                rows: (close) => rows(() => close(null)),
              ),
            ),
            const SizedBox(width: 6),
          ],
          if (widget.removeButton) ...[
            RowButton(
              key: ValueKey<String>(
                  '${widget.keyPrefix}-remove-button-${widget.id}'),
              label: l10n.shapeOpRemove,
              onTap: (_) => widget.onDelete(),
            ),
            const SizedBox(width: 6),
          ],
          SizedBox(
            width: widget.valueColumn.width,
            child: Row(
              mainAxisAlignment: MainAxisAlignment.end,
              children: [
                if (widget.opacity case final opacity?)
                  SizedBox(
                    width: 56,
                    // Staged and previewed, like every other dragged value
                    // here: the drag shows live and commits once on release,
                    // so it is one op and one undo step.
                    child: DragValueField(
                      key: ValueKey<String>(
                          '${widget.keyPrefix}-opacity-${widget.id}'),
                      value: _staged ?? opacity,
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
                        _preview(opacity);
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
        if (widget.onRename != null && widget.menuRows != null)
          const MenuDivider(),
        ...?widget.menuRows?.call(() => close(null)),
        const MenuDivider(),
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

/// A small word-button on an item row, in the accent: Add, with the arrow
/// After Effects draws after it, or Remove.
class RowButton extends StatelessWidget {
  final String label;
  final bool arrow;
  final void Function(Offset at) onTap;

  const RowButton({
    super.key,
    required this.label,
    required this.onTap,
    this.arrow = false,
  });

  @override
  Widget build(BuildContext context) {
    final t = ThemeScope.of(context).theme;
    return HoverMark(
      builder: (context, lit) {
        final colour = Color.lerp(t.accent, t.accentHover, lit)!;
        return GestureDetector(
          behavior: HitTestBehavior.opaque,
          onTapUp: (details) => onTap(details.globalPosition),
          child: Row(
            mainAxisSize: MainAxisSize.min,
            children: [
              // A narrow heading shortens the label rather than overflowing.
              Flexible(
                child: Text(label,
                    style: t.small.copyWith(color: colour),
                    maxLines: 1,
                    overflow: TextOverflow.ellipsis),
              ),
              if (arrow) ...[
                const SizedBox(width: 2),
                glyph.LumitIcon(LumitIcons.expand,
                    size: iconSize, colour: colour),
              ],
            ],
          ),
        );
      },
    );
  }
}
