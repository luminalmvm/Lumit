// A title or a solid put down from the Cut timeline: an ordinary layer of its
// own, five seconds long, at the frame asked for.

import 'dart:math' show max;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:provider/provider.dart';

import '../l10n/strings.dart';
import '../shell/layer_settings_frb.dart' show newSolidFrb;
import '../state/dock.dart';
import '../state/text_documents.dart';
import '../state/tools.dart';
import '../widgets/controls.dart';
import 'layer_fold_frb.dart';
import 'viewer_type.dart' show textAnchor, viewerTypeEditRequest;

const int _insertSeconds = 5;

/// [layer] over five seconds from [frame], with its own clock starting there.
/// A composition too short for it grows, as it does for a clip put past its
/// end.
void _place(CompositionReference comp, LayerReference layer, int frame) {
  final settings = comp.getSettings();
  final (fpsNum, fpsDen) = (settings.fpsNum, settings.fpsDen);
  final frames = (_insertSeconds * fpsNum / max(1, fpsDen)).round();
  final start = timeOfSubframe(frame.toDouble(), fpsNum, fpsDen);
  final end = timeOfSubframe((frame + frames).toDouble(), fpsNum, fpsDen);
  layer.setSpan(
      span: BridgeSpan(inPoint: start, outPoint: end, startOffset: start));
  final length = settings.duration.num / max(1, settings.duration.den);
  if (end.num / end.den <= length) return;
  comp.setSettings(
    settings: BridgeCompSettings(
      name: settings.name,
      width: settings.width,
      height: settings.height,
      fpsNum: fpsNum,
      fpsDen: fpsDen,
      duration: end,
      background: settings.background,
      shutterAngle: settings.shutterAngle,
      motionBlurSamples: settings.motionBlurSamples,
      motionBlurLimit: settings.motionBlurLimit,
    ),
  );
}

/// A Text layer reading the composition's name, in the middle of the frame, at
/// [frame], above row [row]. One undo step. The Text panel comes to the front
/// and the words open for typing in the Viewer, all of them picked, so a first
/// key replaces them.
void insertTitle(BuildContext context, LumitUiState ui,
    CompositionReference comp, int frame, int? row) {
  final app = Provider.of<LumitState>(context, listen: false);
  final settings = comp.getSettings();
  LayerReference? made;
  asOneUndoStep(app.project, () {
    try {
      final layer = comp.addTextLayer(row: row);
      final document = layer.getText()?.copyWith(text: settings.name);
      if (document != null) {
        final anchor = textAnchor(document);
        layer.setTextPlaced(
          document: document,
          anchorX: anchor.dx,
          anchorY: anchor.dy,
          positionX: settings.width / 2,
          positionY: settings.height / 2,
        );
        layer.rename(name: settings.name);
      }
      _place(comp, layer, frame);
      made = layer;
    } catch (_) {}
  });
  final layer = made;
  if (layer == null) return;
  ui.setSelection([layer]);
  app.notifyDocumentChanged();
  ui.model.refresh();
  setPanelVisible(ui.split, Panel.text, true);
  activatePanelTab(ui.split, Panel.text);
  ui.workspace.touch();
  // The Viewer has to have the layer's box before it can open it, and it reads
  // the request as the Type tool arrives, so a Type tool already in hand is
  // put down first.
  final tools = ui.tools;
  if (tools.tool == ToolMode.typeHorizontal) tools.select(ToolMode.select);
  WidgetsBinding.instance.addPostFrameCallback((_) {
    viewerTypeEditRequest = (view: null, layer: layer, at: null);
    tools.select(ToolMode.typeHorizontal);
    WidgetsBinding.instance
        .addPostFrameCallback((_) => viewerTypeEditRequest = null);
  });
}

/// The Insert title and Insert solid rows, for the ground and gap menus.
List<Widget> cutInsertRows(
  BuildContext context,
  LumitUiState ui,
  CompositionReference comp, {
  required VoidCallback close,
  required int frame,
  required int? row,
  required VoidCallback onDone,
}) =>
    [
      MenuRow(
        key: const ValueKey('insert-title'),
        onPressed: () {
          close();
          insertTitle(context, ui, comp, frame, row);
          onDone();
        },
        child: MenuLabel(l10n.insertTitle, action: 'cut.insert.title'),
      ),
      MenuRow(
        key: const ValueKey('insert-solid'),
        onPressed: () {
          close();
          // The Solid dialogue, then the same placement as a title.
          newSolidFrb(
            context: context,
            comp: comp,
            row: row,
            place: (layer) => _place(comp, layer, frame),
            onDone: onDone,
          );
        },
        child: MenuLabel(l10n.insertSolid),
      ),
    ];
