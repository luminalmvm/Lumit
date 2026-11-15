// A colourist's memories: eight numbered grades kept with the project. Alt and
// a digit lays memory n on the current shot, or every picked shot; Mod+Alt and
// the digit keeps the current shot's grade in it. The Gallery shows them as a
// strip of slots above the stills.

import 'dart:async';
import 'dart:convert';
import 'dart:typed_data';
import 'dart:ui' as dui;

import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/src/rust/api/grade_memories.dart';

import '../l10n/strings.dart';
import '../state/grade_target.dart';
import 'grade_clipboard.dart';

/// The slots there are.
const int gradeMemorySlots = 8;

/// The project's filled slots, read once per document revision.
(Object?, int?, List<BridgeGradeMemory>)? _read;

List<BridgeGradeMemory> gradeMemories(LumitState app, LumitUiState ui) {
  final project = app.project;
  if (project == null) return const [];
  final revision = ui.model.revision;
  final held = _read;
  if (held != null && identical(held.$1, project) && held.$2 == revision) {
    return held.$3;
  }
  List<BridgeGradeMemory> read;
  try {
    read = project.getGradeMemories();
  } catch (_) {
    read = const [];
  }
  _read = (project, revision, read);
  return read;
}

BridgeGradeMemory? gradeMemory(LumitState app, LumitUiState ui, int slot) =>
    gradeMemories(app, ui).where((m) => m.slot == slot).firstOrNull;

/// Memory [slot]'s grade in place of the grade on every picked shot. False
/// when the slot is empty or nothing can take it.
bool applyMemory(LumitState app, LumitUiState ui, int slot) {
  final memory = gradeMemory(app, ui, slot);
  if (memory == null || gradeTargets(ui).isEmpty) return false;
  layGrade(app, ui, memory.grade, name: 'Apply memory');
  return true;
}

/// The current shot's grade kept in [slot], with a small picture of the shot
/// at the playhead. False when the shot has no grade.
bool storeMemory(LumitState app, LumitUiState ui, int slot) {
  final shot = ui.gradeShot.value;
  final project = app.project;
  if (shot == null || project == null) return false;
  final ids = shotGradeIds(ui, shot);
  if (ids.isEmpty) return false;
  final String grade;
  try {
    grade = shot.layer.copyEffects(effects: ids);
  } catch (_) {
    return false;
  }
  unawaited(() async {
    final picture = await _picture(ui, shot);
    try {
      project.setGradeMemory(slot: slot, grade: grade, picture: picture);
    } catch (_) {
      return;
    }
    app.notifyDocumentChanged();
    ui.model.refresh();
    app.postNotice(l10n.gradeMemoryKept(slot));
  }());
  return true;
}

/// Empty [slot].
void clearMemory(LumitState app, LumitUiState ui, int slot) {
  try {
    app.project?.setGradeMemory(slot: slot, grade: null, picture: '');
  } catch (_) {
    return;
  }
  app.notifyDocumentChanged();
  ui.model.refresh();
}

/// The shot at the playhead as a small PNG, base64, or empty when there is
/// no picture to take.
Future<String> _picture(LumitUiState ui, GradeShot shot) async {
  final comp = ui.selectedComp;
  if (comp == null) return '';
  try {
    // The grade is kept whether or not a picture comes back in good time.
    final got = await comp
        .shotThumbnail(
            layer: shot.layer.internallayerId,
            clip: shot.clip,
            frame: ui.playheadFrame.value,
            maxEdge: 160)
        .timeout(const Duration(seconds: 2), onTimeout: () => null);
    if (got == null || got.width == 0) return '';
    final done = Completer<dui.Image>();
    dui.decodeImageFromPixels(got.rgba, got.width, got.height,
        dui.PixelFormat.rgba8888, done.complete);
    final image = await done.future;
    final png = await image.toByteData(format: dui.ImageByteFormat.png);
    image.dispose();
    return png == null ? '' : base64Encode(png.buffer.asUint8List());
  } catch (_) {
    return '';
  }
}

/// Memory pictures decoded, by slot and the text they came from.
final Map<int, (String, Uint8List)> _bytes = {};

/// [memory]'s picture as PNG bytes, decoded once, or null when it has none.
Uint8List? gradeMemoryPicture(BridgeGradeMemory memory) {
  if (memory.picture.isEmpty) return null;
  final held = _bytes[memory.slot];
  if (held != null && held.$1 == memory.picture) return held.$2;
  try {
    final bytes = base64Decode(memory.picture);
    _bytes[memory.slot] = (memory.picture, bytes);
    return bytes;
  } catch (_) {
    return null;
  }
}
