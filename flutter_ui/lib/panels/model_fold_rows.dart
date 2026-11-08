// A Model layer's own rows in its fold-out: the Animation group, and the file's
// node tree under Parts. Every row is an ordinary effect row, so it keys, drags
// and eases as any does.

import 'package:lumit_flutter/l10n/engine_labels.dart';
import 'package:lumit_flutter/l10n/strings.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/model_rows.dart';
import 'package:uuid/uuid.dart';

import 'effect_param_row_frb.dart'
    show EffectStackEditor, cachedListParameters, relistedRows;
import 'graph_panel.dart' show DrivenParam;
import 'layer_fold_frb.dart';

/// The path of a Model layer's Animation group, of its Parts group, and of one
/// part in it, each part directly under Parts however deep it sits in the file.
String modelAnimationPath(String layerId) => '$layerId/animation';
String modelPartsPath(String layerId) => '$layerId/parts';
String modelPartPath(String layerId, String partId) =>
    '${modelPartsPath(layerId)}/$partId';

/// The path of a part's own rows, a heading of their own under the part, so the
/// tree stays a line a node however much of it is open.
String modelPartRowsPath(String layerId, String partId) =>
    '${modelPartPath(layerId, partId)}/rows';

/// How many steps a part may indent, so a long chain of bones keeps its names
/// on screen.
const _deepestIndent = 6;

/// The rows [part] shows: what it keeps, or as nobody has changed them.
BridgeEffectInstanceInfo? partRowsOf(
        BridgeModelRows rows, BridgeModelPart part) =>
    part.rows ??
    switch (rows.untouched) {
      final u? => BridgeEffectInstanceInfo(
          id: part.id,
          name: u.name,
          customName: u.customName,
          enabled: u.enabled,
          audio: u.audio,
          values: u.values,
          linkedPairs: u.linkedPairs,
          badgeReason: u.badgeReason,
          badgeDetail: u.badgeDetail,
          derivedParams: u.derivedParams,
          hiddenRows: u.hiddenRows,
          disabledRows: u.disabledRows,
          rowOptions: u.rowOptions,
          rowLabels: u.rowLabels,
          nodeGraphComp: u.nodeGraphComp,
        ),
      null => null,
    };

/// Whether [part]'s Visible row reads shown, `null` when it is keyed or driven,
/// which leaves that to the row itself.
bool? _shown(BridgeModelPart part) {
  final rows = part.rows;
  if (rows == null) return true;
  for (final v in rows.values) {
    if (v.id != 'visible') continue;
    return switch (v.value) {
      BridgeEffectValue_Float(field0: BridgeScalar_Static(:final field0)) =>
        field0 >= 0.5,
      _ => null,
    };
  }
  return true;
}

/// Every instance of [rows] by id, for the graph and the panels that find a row
/// by its path.
BridgeEffectInstanceInfo? modelRowsById(BridgeModelRows rows, String id) {
  if (rows.playback case final p? when p.id.toString() == id) return p;
  for (final part in rows.parts) {
    if (part.id.toString() == id) return partRowsOf(rows, part);
  }
  return null;
}

/// The paths to open to see part [partId] under layer [layerId]: Parts and each
/// part above it, then its own.
List<String> modelPartReveal(
    String layerId, BridgeModelRows rows, String partId) {
  final at = rows.parts.indexWhere((p) => p.id.toString() == partId);
  if (at < 0) return const [];
  final paths = [modelPartPath(layerId, partId)];
  var depth = rows.parts[at].depth;
  for (var i = at - 1; i >= 0 && depth > 0; i--) {
    if (rows.parts[i].depth < depth) {
      depth = rows.parts[i].depth;
      paths.add(modelPartPath(layerId, rows.parts[i].id.toString()));
    }
  }
  return [
    modelPartsPath(layerId),
    ...paths.reversed,
    modelPartRowsPath(layerId, partId),
  ];
}

/// The part [path] names, as one of [layerId]'s Parts paths, or null.
String? modelPartOfPath(String layerId, String path) {
  final head = '${modelPartsPath(layerId)}/';
  return path.startsWith(head) ? path.substring(head.length) : null;
}

/// Set a part's Visible row on [layer], as one undo step.
void setModelPartShown(LayerReference layer, String part, bool shown) {
  EffectStackEditor().write(layer, UuidValue.fromString(part), 'visible',
      BridgeEffectValue.float(BridgeScalar.static_(shown ? 1.0 : 0.0)));
}

/// Add [info]'s Animation group and Parts tree to [rows]. A model not read yet
/// has neither.
void addModelFoldRows({
  required List<LayerFoldRow> rows,
  required BridgeLayerInfo info,
  required String id,
  required Set<String> open,
  required Map<String, DrivenParam> driven,
}) {
  final model = info.modelRows;
  if (model == null) return;
  if (model.playback case final playback?) {
    final path = modelAnimationPath(id);
    final isOpen = open.contains(path);
    rows.add(FoldGroupRow(
        path: path, label: l10n.foldAnimation, open: isOpen, depth: 1));
    if (isOpen) {
      final values = {for (final v in playback.values) v.id: v.value};
      // A take the file has lost says so, last on the list, picked.
      final options = [
        for (final r in playback.rowOptions)
          if (r.id == 'take' && model.lostTake != null)
            BridgeRowOptions(id: r.id, options: [
              ...r.options,
              l10n.modelTakeMissing(model.lostTake!),
            ])
          else
            r,
      ];
      for (final param
          in relistedRows(cachedListParameters(playback.name), options)) {
        rows.add(FoldEffectParamRow(playback, param, values[param.id],
            depth: 2,
            model: true,
            driven: driven['${playback.id}/${param.id}']));
      }
    }
  }
  if (model.parts.isEmpty) return;
  final path = modelPartsPath(id);
  final isOpen = open.contains(path);
  rows.add(
      FoldGroupRow(path: path, label: l10n.foldParts, open: isOpen, depth: 1));
  if (!isOpen) return;
  // The depth of a shut part whose children are being passed over.
  int? shutUnder;
  for (final (i, part) in model.parts.indexed) {
    if (shutUnder != null && part.depth > shutUnder) continue;
    shutUnder = null;
    final partPath = modelPartPath(id, part.id.toString());
    final partOpen = open.contains(partPath);
    final depth = 2 + (part.depth < _deepestIndent ? part.depth : _deepestIndent);
    rows.add(FoldGroupRow(
      path: partPath,
      // A part its file gives no name is numbered.
      label: part.name.isEmpty ? '${engineLabel('Part')} ${i + 1}' : part.name,
      open: partOpen,
      depth: depth,
      named: part.name.isNotEmpty,
      shown: _shown(part),
    ));
    if (!partOpen) {
      shutUnder = part.depth;
      continue;
    }
    // A skin places what it draws, so this part's transform would move nothing.
    if (part.skinned) continue;
    final rowsPath = modelPartRowsPath(id, part.id.toString());
    final rowsOpen = open.contains(rowsPath);
    rows.add(FoldGroupRow(
        path: rowsPath,
        label: l10n.transformSection,
        open: rowsOpen,
        depth: depth + 1));
    // Nothing kept and no path to open means nothing to show the filter that
    // walks every twirl.
    if (!rowsOpen || (part.rows == null && identical(open, everyFoldPath))) {
      continue;
    }
    final own = partRowsOf(model, part);
    if (own == null) continue;
    final values = {for (final v in own.values) v.id: v.value};
    for (final param in cachedListParameters(own.name)) {
      rows.add(FoldEffectParamRow(own, param, values[param.id],
          depth: depth + 2,
          model: true,
          driven: driven['${own.id}/${param.id}']));
    }
  }
}
