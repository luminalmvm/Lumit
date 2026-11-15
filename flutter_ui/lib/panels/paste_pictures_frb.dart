// Paste, when what is on the system clipboard is a picture.
//
// A picture copied in a browser, a screenshot tool or an editor, or picture
// files copied in a file manager, are imported as footage. With the Project
// panel focused that is all that happens. Anywhere else with a composition
// open, each also lands on a layer at the playhead, as a drop on the Timeline
// does. The Cut timeline takes the import and leaves the placing to the user.

import 'dart:io';
import 'dart:math' show max, min;
import 'dart:typed_data' show Uint8List;

import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart' show BridgeRational;
import 'package:lumit_flutter/src/rust/api/footage.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/project.dart';
import 'package:lumit_flutter/src/rust/api/project_item.dart';

import '../l10n/strings.dart';
import '../shell/model_settings_dialog.dart' show modelLightNames;
import '../state/clipboard_image.dart';
import '../state/dock.dart';
import '../state/workspace.dart' show Workspace;
import 'layer_fold_frb.dart' show asOneUndoStep, timeOfSubframe;

/// Import what the system clipboard holds, when it holds pictures. [row] is
/// where a new layer goes in the stack. False when there was nothing to take,
/// so the chord can be left to whoever wants it next.
Future<bool> pasteSystemPictures(
  LumitState app,
  LumitUiState ui,
  CompositionReference? comp, {
  int? row,
}) async {
  if (app.project == null) return false;
  var paths = await clipboardPictureFiles();
  if (paths.isEmpty) {
    final png = await clipboardPicturePng();
    if (png == null) return false;
    final saved = await _savePicture(app.project!, png);
    if (saved == null) {
      app.postNotice(l10n.pasteImageFailed, error: true);
      return true;
    }
    paths = [saved];
  }
  final known = {for (final f in _footage(app.project)) f.internalid};
  if (!await app.importFootagePaths(paths)) return true;
  final fresh = [
    for (final f in _footage(app.project))
      if (!known.contains(f.internalid)) f,
  ];
  if (fresh.isEmpty) return true;
  final onto = ui.activePanel;
  if (comp == null || onto == Panel.project || onto == Panel.cutTimeline) {
    ui.projectPickRequest.value = ItemReference.footage(fresh.first);
    return true;
  }
  final placed = <LayerReference>[];
  try {
    asOneUndoStep(app.project, () {
      final layers = comp.addFootageLayers(
          footage: fresh,
          asSequence: ui.workspace.interface.videoAsSequenceLayer,
          row: row,
          arrival: ui.streamsArrival,
          endToEnd: true,
          lightNames: modelLightNames());
      _startAt(comp, layers, ui.playheadFrame.value);
      placed.addAll(layers);
    });
  } catch (_) {
    // The composition went away while the files were read. They are in the
    // project, which is all that can be said for them now.
  }
  app.notifyDocumentChanged();
  ui.model.refresh();
  if (placed.isNotEmpty) ui.setSelection(placed);
  return true;
}

Iterable<FootageReference> _footage(ProjectReference? project) => [
      for (final item in project?.getItems() ?? const <ItemReference>[])
        if (item case ItemReference_Footage(:final field0)) field0,
    ];

/// Write [png] beside the saved project, or, for a project with no file yet or
/// a folder that will not take it, in Lumit's own folder. Null when neither
/// does.
Future<String?> _savePicture(ProjectReference project, Uint8List png) async {
  final sep = Platform.pathSeparator;
  final saved = project.path();
  for (final home in [
    if (saved != null && saved.isNotEmpty)
      Directory('${File(saved).parent.path}${sep}Pasted images'),
    Directory('${Workspace.storeFile().parent.path}${sep}pasted-images'),
  ]) {
    try {
      return await savePastedPicture(png,
          home: home, name: (n) => l10n.pastedImageName(n));
    } on FileSystemException {
      continue;
    }
  }
  return null;
}

/// Move [layers] on by [frame], keeping their lengths, and cut what then runs
/// past the end of [comp] off, so a still that lasts a whole composition does
/// not hang over it. A layer that starts past the end is left whole.
void _startAt(CompositionReference comp, List<LayerReference> layers, int frame) {
  if (frame <= 0) return;
  final settings = comp.getSettings();
  final (fpsNum, fpsDen) = (settings.fpsNum, settings.fpsDen);
  final fps = fpsNum / max(1, fpsDen);
  double seconds(BridgeRational r) => r.num / max(1, r.den);
  final end = (seconds(settings.duration) * fps).round();
  BridgeRational at(int frames) =>
      timeOfSubframe(frames.toDouble(), fpsNum, fpsDen);
  for (final layer in layers) {
    final span = layer.getSpan();
    final from = (seconds(span.inPoint) * fps).round() + frame;
    final to = (seconds(span.outPoint) * fps).round() + frame;
    final offset = (seconds(span.startOffset) * fps).round() + frame;
    layer.setSpan(
      span: BridgeSpan(
        inPoint: at(from),
        outPoint: at(from < end ? min(to, end) : to),
        startOffset: at(offset),
      ),
    );
  }
}
