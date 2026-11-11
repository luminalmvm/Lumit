// Where a control's row sits in the Timeline's fold: the twirls to open on
// the way to it, and the row itself.

import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/shape.dart';
import 'package:lumit_flutter/src/rust/api/template.dart';

import 'layer_fold_frb.dart';
import 'shape_entries.dart';
import 'transform_rows_frb.dart';

/// The fold paths that show [row] under the layer [entry]: every twirl to
/// open, and the row to pick. A row the fold does not list, a Text layer's
/// words or a layer's visibility, has no path, and the layer alone is shown.
({List<String> open, String? row}) templateRowFold(
    BridgeRowRef row, BridgeLayerEntry entry) {
  const none = (open: <String>[], row: null);
  final id = row.layer.toString();
  final info = entry.info;
  final owner = row.owner;

  ({List<String> open, String? row}) shape(ShapeValue value) {
    if (owner == null) return none;
    final contents = contentsPath(id);
    final entries = info.shapeContents;
    final at = entries.indexWhere((e) => shapeEntryId(e) == owner);
    if (at < 0) return none;
    // The groups the shape sits in, each shut until opened.
    final groups = <String>[];
    var depth = entries[at].depth;
    for (var i = at - 1; i >= 0 && depth > 0; i--) {
      final above = entries[i];
      if (above.depth < depth && above.kind is BridgeShapeEntryKind_Group) {
        groups.add('$contents/${shapeEntryId(above)}');
        depth = above.depth;
      }
    }
    return (
      open: [contents, ...groups.reversed, '$contents/$owner'],
      row: '$contents/$owner/${value.name}',
    );
  }

  ({List<String> open, String? row}) mask(MaskValue value) => owner == null
      ? none
      : (
          open: [masksPath(id)],
          row: '${masksPath(id)}/$owner/${value.name}',
        );

  switch (row.kind) {
    case BridgeRowKind.transform:
      if (row.props.isEmpty) return none;
      final groups = transformGroups(
        threeD: info.switches.threeD,
        modes: info.axisModes,
        kind: info.kind,
        twoNode: info.camera?.twoNode ?? false,
        light: info.light,
        material: info.material,
      );
      for (final group in groups) {
        if (group.axes.any((axis) => axis.prop == row.props.first)) {
          // A transform row shows on its own, with its heading left shut.
          final path = transformGroupPath(id, group);
          return (open: [path], row: path);
        }
      }
      return none;
    case BridgeRowKind.param || BridgeRowKind.paramPoint:
      final param = row.param;
      if (owner == null || param == null) return none;
      final style = info.styles.any((s) => s.id == owner);
      final heading = style
          ? stylePath(id, owner.toString())
          : effectPath(id, owner.toString());
      return (
        open: [style ? stylesPath(id) : effectsPath(id), heading],
        row: '$heading/$param',
      );
    case BridgeRowKind.effectOn:
      if (owner == null) return none;
      return (
        open: [effectsPath(id)],
        row: effectPath(id, owner.toString()),
      );
    case BridgeRowKind.shapeFill:
      return shape(ShapeValue.fillColour);
    case BridgeRowKind.shapeStroke:
      return shape(ShapeValue.strokeColour);
    case BridgeRowKind.shapeStrokeWidth:
      return shape(ShapeValue.strokeWidth);
    case BridgeRowKind.shapeOpacity:
      return shape(ShapeValue.opacity);
    case BridgeRowKind.shapeWidth:
      return shape(ShapeValue.shapeWidth);
    case BridgeRowKind.shapeHeight:
      return shape(ShapeValue.shapeHeight);
    case BridgeRowKind.shapeRoundness:
      return shape(ShapeValue.roundness);
    case BridgeRowKind.shapeRotation:
      return shape(ShapeValue.shapeRotation);
    case BridgeRowKind.maskOpacity:
      return mask(MaskValue.opacity);
    case BridgeRowKind.maskFeather:
      return mask(MaskValue.feather);
    case BridgeRowKind.maskExpansion:
      return mask(MaskValue.expansion);
    case BridgeRowKind.volume:
      return (open: [audioPath(id)], row: '${audioPath(id)}/volume');
    case BridgeRowKind.copy:
      final param = row.param;
      if (param == null) return none;
      return (open: [inputsPath(id)], row: '${inputsPath(id)}/$param');
    case BridgeRowKind.text ||
          BridgeRowKind.font ||
          BridgeRowKind.textSize ||
          BridgeRowKind.textFill ||
          BridgeRowKind.textStroke ||
          BridgeRowKind.textStrokeWidth ||
          BridgeRowKind.textTracking ||
          BridgeRowKind.textBackground ||
          BridgeRowKind.visible ||
          BridgeRowKind.source:
      return none;
  }
}
