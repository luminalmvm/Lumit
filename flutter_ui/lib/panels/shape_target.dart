import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/shape.dart';
import 'package:uuid/uuid.dart';

import '../state/comp_time.dart';
import '../state/tools.dart';
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'layer_fold_frb.dart' show ShapeValue, shapeItemWith, stillColour;
import 'shape_entries.dart';

typedef ShapeTarget = ({
  LayerReference layer,
  List<BridgeShapeEntry> contents,
  Set<UuidValue> items,
});

typedef ShapePaint = ({
  bool fillOn,
  ToolColour fill,
  bool strokeOn,
  ToolColour stroke,
  double width,
});

typedef ShapeEdit = BridgeShapeItem Function(
    BridgeShapeItem item, CompositionReference comp, int frame);

List<ShapeTarget> shapeTargets(LumitUiState ui, {UuidValue? only}) {
  final selected = ui.selectedLayerIds;
  return [
    for (final entry in ui.model.heldLayers)
      if (entry.info.kind == BridgeLayerKind.shape &&
          selected.contains(entry.layer.internallayerId) &&
          (only == null || only == entry.layer.internallayerId) &&
          entry.info.shapeContents.items.isNotEmpty)
        (
          layer: entry.layer,
          contents: entry.info.shapeContents,
          items: _pickedItems(entry.info.shapeContents,
              entry.layer.internallayerId, ui.selectedProperties.value),
        ),
  ];
}

Set<UuidValue> _pickedItems(
    List<BridgeShapeEntry> contents, UuidValue layer, List<String> picks) {
  final named = {
    for (final path in picks)
      if (path.split('/') case [final owner, 'contents', final id, ...]
          when owner == layer.toString())
        id,
  };
  final picked = <UuidValue>{};
  int? within;
  for (final entry in contents) {
    if (within != null && entry.depth <= within) within = null;
    final hit = named.contains(shapeEntryId(entry).toString());
    if (hit && entry.kind is BridgeShapeEntryKind_Group) within ??= entry.depth;
    if ((hit || within != null) && entry.kind is BridgeShapeEntryKind_Item) {
      picked.add(shapeEntryId(entry));
    }
  }
  return picked.isNotEmpty ? picked : {for (final i in contents.items) i.id};
}

BridgeShapeItem shownShape(ShapeTarget target) =>
    target.contents.items.lastWhere((i) => target.items.contains(i.id));

double _now(BridgeScalar s, CompositionReference comp, int frame) =>
    s is BridgeScalar_Static
        ? s.field0
        : sampledScalar(s, timeOfFrame(comp, frame));

ShapePaint paintOfShape(BridgeShapeItem item, ToolsState tools,
    CompositionReference comp, int frame) {
  ToolColour colour(BridgeColour c) => ToolColour(
      _now(c.r, comp, frame), _now(c.g, comp, frame), _now(c.b, comp, frame));
  return (
    fillOn: item.fill != null && !item.fillHidden,
    fill: item.fill == null ? tools.fill : colour(item.fill!),
    strokeOn: item.stroke != null && !item.strokeHidden,
    stroke: item.stroke == null ? tools.stroke : colour(item.stroke!),
    width: _now(item.strokeWidth, comp, frame),
  );
}

ShapePaint paintOfTools(ToolsState tools) => (
      fillOn: tools.fillOn,
      fill: tools.fill,
      strokeOn: tools.strokeOn,
      stroke: tools.stroke,
      width: tools.strokeWidth,
    );

BridgeScalar _moved(
    BridgeScalar scalar, double to, CompositionReference comp, int frame) {
  if (scalar is BridgeScalar_Expression) return scalar;
  if ((_now(scalar, comp, frame) - to).abs() < 1e-6) return scalar;
  return scalarWithValueAt(scalar, to, comp, frame);
}

BridgeColour _recoloured(
    BridgeColour? was, ToolColour to, CompositionReference comp, int frame) {
  if (was == null) {
    return stillColour(BridgeColourRgba(r: to.r, g: to.g, b: to.b, a: 1));
  }
  final r = _moved(was.r, to.r, comp, frame);
  final g = _moved(was.g, to.g, comp, frame);
  final b = _moved(was.b, to.b, comp, frame);
  return identical(r, was.r) && identical(g, was.g) && identical(b, was.b)
      ? was
      : BridgeColour(r: r, g: g, b: b, a: was.a);
}

ShapeEdit fillColourEdit(ToolColour to) => (item, comp, frame) {
      final fill = _recoloured(item.fill, to, comp, frame);
      return identical(fill, item.fill)
          ? item
          : shapeItemWith(item, fill: fill, fillHidden: false);
    };

ShapeEdit strokeColourEdit(ToolColour to) => (item, comp, frame) {
      final stroke = _recoloured(item.stroke, to, comp, frame);
      return identical(stroke, item.stroke)
          ? item
          : shapeItemWith(item, stroke: stroke, strokeHidden: false);
    };

ShapeEdit fillOnEdit(bool on, ToolColour colour) => (item, comp, frame) {
      if (!on) {
        return item.fill == null || item.fillHidden
            ? item
            : shapeItemWith(item, fillHidden: true);
      }
      if (item.fill != null && !item.fillHidden) return item;
      return shapeItemWith(item,
          fill: item.fill ?? _recoloured(null, colour, comp, frame),
          fillHidden: false);
    };

ShapeEdit strokeOnEdit(bool on, ToolColour colour) => (item, comp, frame) {
      if (!on) {
        return item.stroke == null || item.strokeHidden
            ? item
            : shapeItemWith(item, strokeHidden: true);
      }
      if (item.stroke != null && !item.strokeHidden) return item;
      return shapeItemWith(item,
          stroke: item.stroke ?? _recoloured(null, colour, comp, frame),
          strokeHidden: false);
    };

ShapeEdit strokeWidthEdit(double to, ToolColour colour) => (item, comp, frame) {
      final width = _moved(item.strokeWidth, to, comp, frame);
      final gains = item.stroke == null &&
          to > 0 &&
          _now(item.strokeWidth, comp, frame) == 0;
      if (identical(width, item.strokeWidth) && !gains) return item;
      return shapeItemWith(item,
          value: ShapeValue.strokeWidth,
          to: width,
          stroke: gains ? _recoloured(null, colour, comp, frame) : null);
    };

List<BridgeShapeEntry>? editedContents(List<BridgeShapeEntry> contents,
    Set<UuidValue> items, ShapeEdit edit, CompositionReference comp, int frame) {
  var changed = false;
  var next = contents;
  for (final item in contents.items) {
    if (!items.contains(item.id)) continue;
    final to = edit(item, comp, frame);
    if (identical(to, item)) continue;
    changed = true;
    next = next.withItem(item.id, (_) => to);
  }
  return changed ? next : null;
}

bool commitShapes(LumitState state, LumitUiState ui, List<ShapeTarget> targets,
    ShapeEdit edit) {
  final comp = ui.selectedComp;
  if (comp == null) return false;
  final frame = ui.playheadFrame.value;
  final group = targets.length > 1 ? state.project : null;
  var wrote = false;
  group?.beginUndoGroup();
  try {
    for (final target in targets) {
      try {
        final next = editedContents(
            target.layer.getShapeContents(), target.items, edit, comp, frame);
        if (next == null) continue;
        target.layer.setShapeContents(contents: next, at: null);
        wrote = true;
      } catch (_) {
      }
    }
  } finally {
    group?.endUndoGroup();
  }
  if (!wrote) return false;
  state.notifyDocumentChanged();
  ui.model.refresh();
  return true;
}

void previewShapes(LumitUiState ui, ShapeTarget target, ShapeEdit? edit) {
  final comp = ui.selectedComp;
  if (comp == null) return;
  final frame = ui.playheadFrame.value;
  try {
    final contents = target.layer.getShapeContents();
    comp.renderFrameWithShapePreview(
      frame: frame,
      scale: ui.viewerScale,
      layer: target.layer,
      contents: (edit == null
              ? null
              : editedContents(contents, target.items, edit, comp, frame)) ??
          contents,
    );
  } catch (_) {
  }
}
