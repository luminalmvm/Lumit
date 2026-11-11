// Which row of a composition a control of its graphics template can name:
// the ref a row builder hands its menu, the refs of the rows selected in the
// Timeline, and the mark a row wears while it is a control.

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart'
    show BridgeLayerEntry;
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/template.dart';
import 'package:lumit_flutter/src/rust/api/template/copy.dart';
import 'package:provider/provider.dart';
import 'package:uuid/uuid.dart';

import '../icons/lumit_icon.dart' as glyph;
import '../icons/lumit_icons.dart';
import '../l10n/engine_labels.dart';
import '../l10n/strings.dart';
import '../shell/menu_bar_frb.dart' show MenuEntry;
import '../state/layer_bounds.dart' show meshKindOf;
import '../state/template_controls.dart';
import '../widgets/controls.dart' show ThemeScope;
import 'effect_param_row_frb.dart' show instanceParameters;
import 'layer_fold_frb.dart';
import 'transform_rows_frb.dart';

/// A row of [layer] that needs no more than its kind to be named: its words,
/// its footage, whether it shows, its Volume and a Text layer's own rows.
/// [owner] is the effect for [BridgeRowKind.effectOn].
BridgeRowRef layerControlRow(BridgeRowKind kind, UuidValue layer, String label,
        {UuidValue? owner}) =>
    BridgeRowRef(
        kind: kind, layer: layer, props: const [], owner: owner, label: label);

/// A transform row: every axis it shows, in order.
BridgeRowRef transformControlRow(UuidValue layer, TransformGroup group) =>
    BridgeRowRef(
      kind: BridgeRowKind.transform,
      layer: layer,
      props: [for (final axis in group.axes) axis.prop],
      label: group.label,
    );

/// Whether a control can set a row of this kind. A layer picker, a file, a
/// curve and a button hold nothing a copy could be handed.
bool _settable(BridgeParamKind kind) => switch (kind) {
      BridgeParamKind_Float() ||
      BridgeParamKind_Int() ||
      BridgeParamKind_Slider() ||
      BridgeParamKind_Angle() ||
      BridgeParamKind_Choice() ||
      BridgeParamKind_Bool() ||
      BridgeParamKind_Colour() ||
      BridgeParamKind_Text() =>
        true,
      _ => false,
    };

/// One row of an effect or a layer style on [layer]'s own stack, or null for
/// a row of a kind no control sets.
BridgeRowRef? paramControlRow(
        UuidValue layer, UuidValue effect, BridgeParamInfo param) =>
    !_settable(param.kind)
        ? null
        : BridgeRowRef(
            kind: BridgeRowKind.param,
            layer: layer,
            props: const [],
            owner: effect,
            param: param.id,
            label: engineLabel(param.label),
          );

/// An effect's point, the row across and the row down drawn as one.
BridgeRowRef pointControlRow(UuidValue layer, UuidValue effect,
        BridgeParamInfo x, BridgeParamInfo y, String label) =>
    BridgeRowRef(
      kind: BridgeRowKind.paramPoint,
      layer: layer,
      props: const [],
      owner: effect,
      param: x.id,
      paramY: y.id,
      label: label,
    );

/// A row of a copy: the control of the template placed on [layer] that keeps
/// the row [id], which a template placed inside another passes up. Null for a
/// layer that is no copy, and for a row no control of it keeps.
BridgeRowRef? copyControlRow(UuidValue layer, BridgeCopy? copy, String id) {
  if (copy == null) return null;
  for (final control in copy.controls) {
    if (!control.rows.contains(id)) continue;
    return BridgeRowRef(
      kind: BridgeRowKind.copy,
      layer: layer,
      props: const [],
      param: control.id,
      label: control.label,
    );
  }
  return null;
}

/// The row of an effect card or fold: on the layer's own stack or its styles
/// a row of the effect, on a copy's card the template's control. Null on a
/// clip's, a layer group's, a material's, a model's and a driver's rows,
/// which the engine cannot name.
BridgeRowRef? effectControlRow(
  UuidValue layer,
  UuidValue effect,
  BridgeParamInfo param, {
  bool inputs = false,
  BridgeCopy? copy,
  bool elsewhere = false,
}) =>
    elsewhere
        ? null
        : inputs
            ? copyControlRow(layer, copy, param.id)
            : paramControlRow(layer, effect, param);

/// A mask's Opacity, Feather or Expansion. Its path names nothing.
BridgeRowRef? maskControlRow(UuidValue layer, UuidValue mask, MaskValue value) {
  final kind = switch (value) {
    MaskValue.opacity => BridgeRowKind.maskOpacity,
    MaskValue.feather => BridgeRowKind.maskFeather,
    MaskValue.expansion => BridgeRowKind.maskExpansion,
    MaskValue.path || MaskValue.vertexFeather => null,
  };
  return kind == null
      ? null
      : BridgeRowRef(
          kind: kind,
          layer: layer,
          props: const [],
          owner: mask,
          label: maskValueLabel(value),
        );
}

/// A shape item's paint, and a parametric shape's size, corners and turn.
/// Every other shape row names nothing.
BridgeRowRef? shapeControlRow(
    UuidValue layer, UuidValue item, ShapeValue value) {
  final kind = switch (value) {
    ShapeValue.fillColour => BridgeRowKind.shapeFill,
    ShapeValue.strokeColour => BridgeRowKind.shapeStroke,
    ShapeValue.strokeWidth => BridgeRowKind.shapeStrokeWidth,
    ShapeValue.opacity => BridgeRowKind.shapeOpacity,
    ShapeValue.shapeWidth => BridgeRowKind.shapeWidth,
    ShapeValue.shapeHeight => BridgeRowKind.shapeHeight,
    ShapeValue.roundness => BridgeRowKind.shapeRoundness,
    ShapeValue.shapeRotation => BridgeRowKind.shapeRotation,
    _ => null,
  };
  return kind == null
      ? null
      : BridgeRowRef(
          kind: kind,
          layer: layer,
          props: const [],
          owner: item,
          label: shapeValueLabel(value),
        );
}

/// The layer's Volume.
BridgeRowRef volumeControlRow(UuidValue layer) =>
    layerControlRow(BridgeRowKind.volume, layer, l10n.volume);

/// The row a Timeline fold path names, read off [layers], or null for a path
/// no control can name: a heading, a clip's or a group's row, a path gone
/// from the composition.
BridgeRowRef? controlRowOfPath(String path, List<BridgeLayerEntry> layers) {
  final parts = path.split('/');
  if (parts.length < 3) return null;
  final entry = layers
      .where((e) => e.layer.internallayerId.toString() == parts[0])
      .firstOrNull;
  if (entry == null) return null;
  final info = entry.info;
  final layer = entry.layer.internallayerId;
  UuidValue? id(String text) {
    try {
      return UuidValue.withValidation(text);
    } catch (_) {
      return null;
    }
  }

  switch (parts[1]) {
    case 'transform' when parts.length == 3:
      return transformGroups(
        threeD: info.switches.threeD,
        modes: info.axisModes,
        kind: meshKindOf(info),
        twoNode: info.camera?.twoNode ?? false,
        light: info.light,
        material: info.material,
      )
          .where((g) => g.axes.first.prop.name == parts[2])
          .map((g) => transformControlRow(layer, g))
          .firstOrNull;
    case 'effects' || 'styles' when parts.length == 4:
      final stack = parts[1] == 'effects' ? info.effects : info.styles;
      final fx = stack.where((e) => e.id.toString() == parts[2]).firstOrNull;
      if (fx == null) return null;
      final param =
          instanceParameters(fx).where((p) => p.id == parts[3]).firstOrNull;
      return param == null ? null : paramControlRow(layer, fx.id, param);
    case 'inputs' when parts.length == 3:
      return copyControlRow(layer, info.copy, parts[2]);
    case 'masks' when parts.length == 4:
      final value = MaskValue.values.asNameMap()[parts[3]];
      final mask = id(parts[2]);
      return value == null || mask == null
          ? null
          : maskControlRow(layer, mask, value);
    case 'contents' when parts.length == 4:
      final value = ShapeValue.values.asNameMap()[parts[3]];
      final item = id(parts[2]);
      return value == null || item == null
          ? null
          : shapeControlRow(layer, item, value);
    case 'audio' when parts.length == 3 && parts[2] == 'volume':
      return volumeControlRow(layer);
  }
  return null;
}

/// The rows selected in the Timeline that a control can name, each once, in
/// the order they were selected.
List<BridgeRowRef> selectedControlRows(LumitUiState ui) {
  final layers = ui.model.heldLayers;
  final seen = <String>{};
  return [
    for (final path in ui.selectedProperties.value)
      if (controlRowOfPath(path, layers) case final row?)
        if (seen.add(controlRowKey(row))) row,
  ];
}

/// [row], and with it every selected row when it is one of them, so a menu
/// opened on a selection acts on all of it.
List<BridgeRowRef> withSelectedControlRows(LumitUiState ui, BridgeRowRef row) {
  final selected = selectedControlRows(ui);
  final key = controlRowKey(row);
  return selected.any((r) => controlRowKey(r) == key) ? selected : [row];
}

/// Composition ▸ Add to controls, and its shortcut: every selected row a
/// control can name becomes one, as one undo step. Greyed with no such row
/// selected, and inside a node graph.
MenuEntry addToControlsRow(LumitUiState ui) {
  final comp = ui.selectedComp;
  final controls = ui.templateControls;
  final rows = comp == null || !controls.offers(comp)
      ? const <BridgeRowRef>[]
      : selectedControlRows(ui);
  return MenuEntry(
    l10n.controlsAdd,
    rows.isEmpty ? null : () => controls.add(rows),
    action: 'template.controls.add',
  );
}

/// The mark a row wears while it is a control. Nothing on any other row, so
/// a row that is none keeps its whole width for its name.
class ControlMark extends StatelessWidget {
  final BridgeRowRef? row;

  /// A layer's own row, in place of [row]: marked while the layer's
  /// visibility, words or footage is a control.
  final UuidValue? layer;
  const ControlMark(this.row, {super.key}) : layer = null;
  const ControlMark.layer(UuidValue this.layer, {super.key}) : row = null;

  @override
  Widget build(BuildContext context) {
    if (row == null && layer == null) return const SizedBox.shrink();
    // Null outside the shell, where there is no composition to hold one of.
    final controls =
        Provider.of<LumitUiState?>(context, listen: false)?.templateControls;
    if (controls == null) return const SizedBox.shrink();
    return ListenableBuilder(
      listenable: controls,
      builder: (context, _) {
        final marked = switch (row) {
          final row? => controls.controlFor(row) != null,
          null => controls.layerIsControl(layer!),
        };
        if (!marked) return const SizedBox.shrink();
        return Padding(
          padding: const EdgeInsetsDirectional.only(start: 4),
          child: glyph.LumitIcon(LumitIcons.expose,
              size: 12, colour: ThemeScope.of(context).theme.textMuted),
        );
      },
    );
  }
}
