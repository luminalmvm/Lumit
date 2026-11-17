// Graphics templates from the library put to use: brought into the project
// once, then placed as a composition from the Project panel is, in the
// Timeline as a Precomp layer or in the Cut timeline as a clip. And a `.lumt`
// handed to Lumit, put into the library.

import 'dart:io';
import 'dart:math' show max;

import 'package:flutter/widgets.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/api/cut.dart' show BridgeCutResult;
import 'package:lumit_flutter/src/rust/api/effect.dart' show BridgeRational;
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'package:lumit_flutter/src/rust/api/template/library.dart';
import 'package:uuid/uuid.dart';

import '../l10n/strings.dart';
import '../shell/menu_bar_frb.dart' show newLayerRow;
import '../shell/settings_account.dart' show askBefore;
import '../state/copies.dart' show focusCopyWords;
import '../state/dock.dart';
import '../state/template_library.dart';
import 'layer_fold_frb.dart' show timeOfSubframe;

/// How long Insert title makes a template that has an intro or an outro.
const int _titleSeconds = 5;

/// Bring the library template at [path] into the open project, filed in the
/// Graphics templates folder, and answer its composition. A project that has
/// that version already answers the one it has.
Future<CompositionReference?> bringTemplate(LumitState app, String path) async {
  final project = app.project;
  if (project == null) return null;
  if (!File(path).existsSync()) {
    app.postNotice(l10n.templateGone, error: true);
    return null;
  }
  try {
    return await project.placeTemplate(
        path: path, folder: l10n.templatesFolder);
  } catch (_) {
    app.postNotice(l10n.templatePlaceFailed, error: true);
    return null;
  }
}

/// **Open**: bring the template in and front its composition for editing.
Future<void> openTemplate(LumitState app, LumitUiState ui, String path) async {
  final comp = await bringTemplate(app, path);
  if (comp == null) return;
  app.notifyDocumentChanged();
  ui.setSelectedComp(comp);
}

/// The template at [path] as a Precomp layer in [comp] at [row], as a
/// composition dropped on the Timeline lands. With [atFrame] its first frame
/// is there. One undo step, and the new copy is selected.
Future<void> placeTemplateInTimeline(LumitState app, LumitUiState ui,
    CompositionReference comp, String path,
    {int? row, int? atFrame}) async {
  final project = app.project;
  if (project == null) return;
  LayerReference? made;
  project.beginUndoGroup();
  try {
    final template = await bringTemplate(app, path);
    if (template == null) return;
    final layer = comp.addPrecompLayer(comp: template, row: row);
    made = layer;
    if (atFrame != null) _startAt(comp, layer, atFrame);
  } catch (_) {
    // A composition cannot place itself. The engine refuses and nothing is
    // placed.
  } finally {
    project.endUndoGroup();
  }
  app.notifyDocumentChanged();
  ui.model.refresh();
  if (made case final layer?) ui.setSelection([layer]);
}

/// Move [layer] so its in point is at [frame], keeping its length.
void _startAt(CompositionReference comp, LayerReference layer, int frame) {
  final settings = comp.getSettings();
  final (fpsNum, fpsDen) = (settings.fpsNum, settings.fpsDen);
  final fps = fpsNum / max(1, fpsDen);
  final span = layer.getSpan();
  double seconds(BridgeRational r) => r.num / max(1, r.den);
  final length =
      ((seconds(span.outPoint) - seconds(span.inPoint)) * fps).round();
  final lead =
      ((seconds(span.startOffset) - seconds(span.inPoint)) * fps).round();
  layer.setSpan(
    span: BridgeSpan(
      inPoint: timeOfSubframe(frame.toDouble(), fpsNum, fpsDen),
      outPoint: timeOfSubframe((frame + length).toDouble(), fpsNum, fpsDen),
      startOffset: timeOfSubframe((frame + lead).toDouble(), fpsNum, fpsDen),
    ),
  );
}

/// The template at [path] as one clip in [comp] at [frame], on [target] or a
/// new track, as a composition dropped on the Cut timeline lands. With
/// [title] a template that has an intro or an outro is made five seconds
/// long. One undo step; the new clip is picked. Answers it, or null when
/// nothing was placed.
Future<({LayerReference layer, UuidValue clip})?> placeTemplateInCut(
    LumitState app, LumitUiState ui, CompositionReference comp, String path,
    {LayerReference? target, required int frame, bool title = false}) async {
  final project = app.project;
  if (project == null) return null;
  var result = BridgeCutResult.nothing;
  ({LayerReference layer, UuidValue clip})? made;
  project.beginUndoGroup();
  try {
    final template = await bringTemplate(app, path);
    if (template == null) return null;
    final before = {
      for (final clip in target?.getClips() ?? const <BridgeClip>[]) clip.id,
    };
    result = comp.cutPlaceComp(target: target, comp: template, atFrame: frame);
    if (result == BridgeCutResult.done) {
      // A new track goes to the top of the stack.
      final layer = target ?? comp.getLayers().first;
      final clip = layer
          .getClips()
          .where((c) => !before.contains(c.id))
          .firstOrNull;
      if (clip != null) {
        made = (layer: layer, clip: clip.id);
        final marks = template.getTemplate();
        if (title && (marks?.intro != null || marks?.outro != null)) {
          final settings = comp.getSettings();
          final frames = (_titleSeconds *
                  settings.fpsNum /
                  max(1, settings.fpsDen))
              .round();
          comp.cutStretch(
              clip: clip.id,
              startFrame: clip.startFrame,
              endFrame: clip.startFrame + frames,
              ripple: false,
              linked: false);
        }
      }
    }
  } catch (_) {
    // A clip that went away between the draw and the press.
  } finally {
    project.endUndoGroup();
  }
  ui.reportCut(result);
  if (result != BridgeCutResult.done) return null;
  app.notifyDocumentChanged();
  ui.model.refresh();
  if (made case final picked?) {
    // The Cut timeline lights it, and Effect controls shows its card.
    ui.cutPickRequest.value = {picked.clip.toString()};
    ui.cutClip.value = picked;
    ui.setSelection([picked.layer]);
  }
  return made;
}

/// The picture track a template placed at the playhead in the Cut timeline
/// goes on: the picked clip's, else the selected layer when it is one. Null
/// makes a new track.
LayerReference? _pictureTrack(LumitUiState ui) {
  final picked = ui.cutClip.value?.layer ?? ui.selectedLayer.value;
  if (picked == null) return null;
  final entry = ui.model.byId(picked.internallayerId);
  return entry?.info.kind == BridgeLayerKind.sequence ? picked : null;
}

/// Double-click, `Enter` or **Place** on a tile: at the playhead, in
/// whichever of the Timeline and the Cut timeline had the keys last.
Future<void> placeTemplateAtPlayhead(
    LumitState app, LumitUiState ui, String path) async {
  final comp = ui.selectedComp;
  if (comp == null) {
    app.postNotice(l10n.selectACompositionFirst);
    return;
  }
  final frame = ui.playheadFrame.value;
  if (TemplateUpdates.of(app, ui).lastTimeline == Panel.cutTimeline) {
    await placeTemplateInCut(app, ui, comp, path,
        target: _pictureTrack(ui), frame: frame);
  } else {
    await placeTemplateInTimeline(app, ui, comp, path,
        row: newLayerRow(ui, comp), atFrame: frame);
  }
}

/// Insert title with a template chosen for it: the template as a clip at
/// [frame] on the selected picture track, its first words ready to type.
/// False when none is chosen or its file has left the library, and Insert
/// title makes a Text layer as it always has.
bool insertTitleTemplate(LumitState app, LumitUiState ui,
    CompositionReference comp, int frame) {
  final path = ui.workspace.titleTemplate;
  if (path.isEmpty || !File(path).existsSync()) return false;
  () async {
    final made = await placeTemplateInCut(app, ui, comp, path,
        target: _pictureTrack(ui), frame: frame, title: true);
    if (made == null) return;
    // Effect controls has to have read the new clip before its card can
    // take the cursor.
    WidgetsBinding.instance.addPostFrameCallback(
        (_) => focusCopyWords(ui, made.layer, clip: made.clip));
  }();
  return true;
}

/// What a `.lumt` is called before it has been read: its file's name.
String _stem(String path) {
  final name = path.split(RegExp(r'[/\\]')).last;
  final dot = name.lastIndexOf('.');
  return dot <= 0 ? name : name.substring(0, dot);
}

/// Put each `.lumt` in [paths] into the library, asking first before one
/// takes the place of another template of its name. Each one added is said
/// on the status line.
Future<void> installTemplateFiles(BuildContext context, LumitState app,
    LumitUiState ui, Iterable<String> paths) async {
  for (final path in paths) {
    final name = _stem(path);
    String? at;
    try {
      at = await installTemplate(path: path, replace: false);
      if (at == null) {
        if (!context.mounted) return;
        if (!await askBefore(context, l10n.templateReplace(name),
            l10n.gradeLookReplaceYes)) {
          continue;
        }
        at = await installTemplate(path: path, replace: true);
      }
    } catch (_) {
      app.postNotice(l10n.templateNotAdded(name), error: true);
      continue;
    }
    if (at == null) continue;
    ui.presetsChanged.value++;
    final added = listTemplates().where((t) => t.path == at).firstOrNull;
    app.postNotice(l10n.templateAdded(added?.name ?? name));
  }
}
