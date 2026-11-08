// Fit to comp: Scale and Position written so a layer's box meets the frame.
//
// Worked from the box the Viewer draws, through the parent chain, flat. A 3D
// layer is fitted on its own plane as if seen square on, since a camera may
// see it from anywhere.

import 'dart:math' as math;

import 'package:flutter/services.dart' show HardwareKeyboard;
import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/orient.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../shell/menu_bar_frb.dart' show MenuEntry;
import '../shell/menu_layer_frb.dart' show onSelection;
import '../state/comp_time.dart';
import 'keyframe_controls_frb.dart' show scalarWithValueAt;
import 'viewer_layer_map.dart';
import 'viewer_stage.dart' show parentMatrix;

enum LayerFit { inside, fill, width, height }

/// Whether Fit to comp has anything to work on: a box, and a Scale and Position
/// an expression isn't driving.
bool canFit(BridgeLayerEntry entry) {
  final tf = entry.info.transform;
  return !const {
        BridgeLayerKind.camera,
        BridgeLayerKind.light,
        BridgeLayerKind.audio,
        BridgeLayerKind.bus,
        BridgeLayerKind.nullLayer,
      }.contains(entry.info.kind) &&
      ![tf.scaleX, tf.scaleY, tf.positionX, tf.positionY]
          .any((s) => s is BridgeScalar_Expression);
}

void _fit(
  BridgeLayerEntry entry,
  LayerFit how,
  LumitUiState ui,
  CompositionReference comp,
) {
  final frame = ui.playheadFrame.value;
  final at = timeOfFrame(comp, frame);
  final revision = ui.model.heldRevision;
  double read(BridgeScalar s, LayerReference layer, BridgeTransformProp prop) =>
      switch (s) {
        BridgeScalar_Static(:final field0) => field0,
        BridgeScalar_Expression() => sampledExpression(s, at, layer, revision,
            row: BridgeExpressionRow.transform(prop)),
        _ => sampledScalar(s, at),
      };
  final byId = <UuidValue, BridgeLayerEntry>{
    for (final e in ui.model.heldLayers) e.layer.internallayerId: e
  };
  BridgeOrientTurn? turnOf(BridgeLayerEntry e) => ui.orientTurns.of(e,
      frame: frame, revision: revision, generation: ui.solveLanded.value);
  final parent = parentMatrix(entry, byId, (e) => e.info.transform, read,
          time: () => at, orientTurn: turnOf)
      ?.storage;
  final tf = entry.info.transform;
  double value(BridgeScalar s, BridgeTransformProp p) =>
      read(s, entry.layer, p);
  final (sx, sy) = (
    value(tf.scaleX, BridgeTransformProp.scaleX),
    value(tf.scaleY, BridgeTransformProp.scaleY),
  );
  final (px, py) = (
    value(tf.positionX, BridgeTransformProp.positionX),
    value(tf.positionY, BridgeTransformProp.positionY),
  );
  // Comp pixels: the view at one to one, from the frame's corner.
  final map = ViewerLayerMap.of(
    positionX: px,
    positionY: py,
    anchorX: value(tf.anchorX, BridgeTransformProp.anchorX),
    anchorY: value(tf.anchorY, BridgeTransformProp.anchorY),
    scaleXPercent: sx,
    scaleYPercent: sy,
    rotationDegrees: value(tf.rotation, BridgeTransformProp.rotation) +
        (turnOf(entry)?.degrees ?? 0),
    origin: Offset.zero,
    viewScale: 1,
    parent: parent == null
        ? null
        : (
            a: parent[0],
            b: parent[1],
            c: parent[4],
            d: parent[5],
            e: parent[12],
            f: parent[13],
          ),
  );
  final size = comp.getSize();
  final bounds = ui.layerBounds.boundsOf(entry,
      compSize: size, revision: revision, t: frame / ui.model.heldFps);
  final corners = [
    map.toScreen(0, 0),
    map.toScreen(bounds.width, 0),
    map.toScreen(bounds.width, bounds.height),
    map.toScreen(0, bounds.height),
  ];
  final xs = corners.map((c) => c.dx);
  final ys = corners.map((c) => c.dy);
  final box = Rect.fromLTRB(xs.reduce(math.min), ys.reduce(math.min),
      xs.reduce(math.max), ys.reduce(math.max));
  if (box.width <= 0 || box.height <= 0) return;
  final (across, down) = (
    size.width / box.width,
    size.height / box.height,
  );
  final k = switch (how) {
    LayerFit.inside => math.min(across, down),
    LayerFit.fill => math.max(across, down),
    LayerFit.width => across,
    LayerFit.height => down,
  };
  // The box grows about the anchor, so the anchor goes where it needs to be
  // for the grown box to sit in the middle of the frame.
  final anchor = map.parentToScreen(px, py);
  final middle = Offset(size.width / 2, size.height / 2);
  final place = map.parentOf(middle - (box.center - anchor) * k);
  entry.layer.setTransforms(
    props: const [
      BridgeTransformProp.scaleX,
      BridgeTransformProp.scaleY,
      BridgeTransformProp.positionX,
      BridgeTransformProp.positionY,
    ],
    values: [
      scalarWithValueAt(tf.scaleX, sx * k, comp, frame),
      scalarWithValueAt(tf.scaleY, sy * k, comp, frame),
      scalarWithValueAt(tf.positionX, place.dx, comp, frame),
      scalarWithValueAt(tf.positionY, place.dy, comp, frame),
    ],
  );
}

/// Fit to comp, width and height, and fill, for the selection. Shift on Fit to
/// comp fills, as the row below it does.
List<MenuEntry> fitRows(LumitState app, LumitUiState ui) {
  final comp = ui.selectedComp;
  VoidCallback? fit(LayerFit Function() how) => comp == null
      ? null
      : onSelection(app, ui, (entry) => _fit(entry, how(), ui, comp),
          when: canFit);
  return [
    MenuEntry(
      l10n.menuFitToComp,
      fit(() => HardwareKeyboard.instance.isShiftPressed
          ? LayerFit.fill
          : LayerFit.inside),
      action: 'layer.fit',
    ),
    MenuEntry(l10n.menuFillComp, fit(() => LayerFit.fill)),
    MenuEntry(l10n.menuFitToCompWidth, fit(() => LayerFit.width),
        action: 'layer.fit.width'),
    MenuEntry(l10n.menuFitToCompHeight, fit(() => LayerFit.height),
        action: 'layer.fit.height'),
  ];
}
