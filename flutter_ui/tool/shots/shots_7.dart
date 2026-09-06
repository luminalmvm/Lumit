// Manual screenshots, sweep 7: the windows that open over the editor.
//
// new-composition · settings · keymap · export · export-queue ·
// recovery-dialog
//
// Each is opened through the function the application's own menu calls, from a
// context taken out of the live tree — so what is photographed is the dialogue
// the program shows, over the editor the program was showing a moment before.
//
// **One shot the manual asks for is not here, and cannot be.**
//
// * `interpretation.png` wants a footage item's interpretation settings. The
//   glossary and docs/07 §3.2 describe them; nothing in the bridge or the
//   panels offers a place to override a file's rate, alpha or colour space.
//
// `export-queue.png` used to be on that list beside it, because export was a
// single dialogue that wrote one composition. There is a queue now, so the
// shot is taken above — through *Add to queue*, which is the button a reader
// would press.
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1   # PowerShell; LUMIT_SHOTS=1 elsewhere
//   flutter run -d windows -t tool/shots/shots_7.dart

// `_context` is not held across the gaps — it is re-read from the live tree at
// every use, which is what the lint is there to make you do.
// ignore_for_file: use_build_context_synchronously

import 'dart:async';
import 'dart:typed_data';
import 'dart:io';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/export_dialog_frb.dart';
import 'package:lumit_flutter/shell/export_queue_frb.dart';
import 'package:lumit_flutter/shell/recovery_dialog_frb.dart';
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/src/rust/api/assets.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/lib.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'package:lumit_flutter/src/rust/api/layer.dart';
import 'shots_common.dart';

/// A context out of the live tree, for the dialogues that take one. The Project
/// panel's Import button is as good as any: it sits under the same Overlay the
/// menu bar's own callbacks reach, which is what `showLumitModal` needs.
BuildContext get _context => elementByKey('project-import')!;

Future<void> main() async {
  final (state, ui) = await bootLumit();
  final project = state.project!;

  final comp = project.newComposition(
    name: 'Opening titles',
    settings: BridgeCompSettings(
      name: 'Opening titles',
      width: 1920,
      height: 1080,
      fpsNum: 25,
      fpsDen: 1,
      duration: BridgeRational(num: 10, den: 1),
      background: F32Array4(Float32List.fromList([0, 0, 0, 1])),
      shutterAngle: 180,
      motionBlurSamples: 16,
    ),
  );

  for (final file in ['Music.wav', 'Gameplay.mp4', 'Title card.mp4']) {
    comp.addFootageLayer(
      footage: project.importFootage(path: '$fixtures/$file'),
      asSequence: false,
    );
  }
  project.importFootage(path: '$fixtures/Logo.png');

  final title = comp.addTextLayer();
  title.setText(
    document: const BridgeTextDocument(animators: [], pathOffset: BridgeScalar.static_(0), 
      text: 'Northern lights',
      size: 140,
      fill: BridgeColourRgba(r: 1, g: 1, b: 1, a: 1),
    ),
  );
  title.setTransforms(props: const [
    BridgeTransformProp.positionX,
    BridgeTransformProp.positionY,
  ], values: const [
    BridgeScalar.static_(490),
    BridgeScalar.static_(840),
  ]);

  final layers = comp.getLayers();
  for (final (index, name)
      in ['Title', 'Title card', 'Gameplay', 'Music'].indexed) {
    layers[index].rename(name: name);
  }
  layers[1].setTransform(
    prop: BridgeTransformProp.opacity,
    value: const BridgeScalar.static_(55),
  );

  ui.setSelectedComp(comp);
  ui.playheadFrame.value = 48;

  // The sweeps photograph the shell; the welcome screen has its own sweep.
  runApp(shotRoot(LumitAppNew(state, ui, welcome: false)));
  await pause(2);
  await sizeWindow(1720, 1000);
  await pause(7);

  // ---- Shot: the New composition dialogue ---------------------------------
  // Opened from the Project panel's own button, which is one of the four routes
  // into `LumitState.newComposition` and the only one that is a single click.
  await tapKey('project-new-comp', settle: 2);
  await captureUi('new-composition.png', scale: 2, crop: _window());
  await tapKey('comp-cancel', settle: 1.5);

  // ---- Shot: the settings window, on the Appearance page -------------------
  unawaited(showSettingsWindowFrb(_context));
  await pause(2);
  await tapKey('settings-page-appearance', settle: 1.5);
  await captureUi('settings.png', scale: 2, crop: _window());

  // ---- Shot: the keymap editor --------------------------------------------
  // The same window, one page along: the editor is a page of Settings rather
  // than a window of its own, and the sidebar showing where you are is part of
  // what the shortcuts page has to explain.
  await tapKey('settings-page-shortcuts', settle: 2);
  await captureUi('keymap.png', scale: 2, crop: _window());
  await tapKey('settings-close', settle: 1.5);

  // ---- Shot: the export settings ------------------------------------------
  // The destination is set through the dialogue's own picker seam — the same
  // call the Choose… button makes — so the row reads as a file somebody has
  // chosen rather than as "not chosen yet".
  unawaited(showExportDialogFrb(
    context: _context,
    comp: comp,
    picker: () async => 'D:/Projects/Northern lights/Opening titles.mp4',
  ));
  await pause(2);
  await tapKey('export-choose', settle: 1.5);
  await captureUi('export.png', scale: 2, crop: _window(tall: true));

  // ---- Shot: the export queue ---------------------------------------------
  // Queued through the dialogue's own *Add to queue* button, which leaves the
  // item waiting rather than starting it, and the window opened the way the
  // dialogue opens it once something is in there. Nothing is faked: the item
  // in the picture is a real export the engine is holding.
  //
  // This was on the manual's "waiting on the feature" list for as long as
  // export was a single dialogue that wrote one composition. It is not any
  // more.
  await tapKey('export-add-to-queue', settle: 2.5);
  unawaited(showExportQueueFrb(context: _context));
  await pause(2.5);
  await captureUi('export-queue.png', scale: 2, crop: _window());
  // Only the queue window is closed here: *Add to queue* shuts the export
  // dialogue behind it on its way out, so there is nothing left to dismiss.
  await tapKey('export-queue-close', settle: 1.5);

  // ---- Shot: the crash-recovery dialogue -----------------------------------
  // Real autosaves, written by the engine's own rotating autosave beside a real
  // saved project — in a throwaway folder, because a sweep must not leave
  // documents in anybody's work. The dialogue then finds them the way it does
  // after a crash: `listAutosaves` on the project it is about to open.
  final scratch = Directory.systemTemp.createTempSync('lumit-shots-recovery');
  final projectPath = '${scratch.path}/Northern lights.lum';
  await project.save(path: projectPath);
  for (var i = 0; i < 3; i++) {
    project.autosave(projectPath: projectPath, keep: 3);
  }
  unawaited(showRecoveryDialogFrb(
    context: _context,
    state: state,
    projectPath: projectPath,
  ));
  await pause(2.5);
  await captureUi('recovery-dialog.png', scale: 2, crop: _window());

  // Nothing was chosen, so nothing was restored; the temporary project goes.
  try {
    scratch.deleteSync(recursive: true);
  } catch (_) {
    // A file the engine still holds open is not worth failing a sweep over.
  }
  exit(0);
}

/// The floating window currently on screen, with a margin of the scrimmed
/// editor around it — the crop every dialogue shot in this sweep wants.
///
/// Worked out from the window itself rather than guessed, because these five
/// are five different sizes. Named rather than typed because the surface a
/// dialogue opens on is `showLumitModal`'s own private window (a movable,
/// resizable one), and a sweep cannot write that class down.
/// [tall] keeps the full height of the app window and narrows only the sides.
/// The export settings are the one dialogue that draws past the box it lays
/// itself out in — its footer sits below the window's own bottom edge — and a
/// crop taken at the box cuts the EXPORT button off the picture.
Rect _window({bool tall = false}) {
  final root = shotRootKey.currentContext!.findRenderObject()! as RenderBox;
  final screen = Offset.zero & root.size;
  // The span rather than the box: the dialogue's Stack has children wider than
  // its own render object, so a margin measured off that box still landed
  // inside the dialogue and sliced the close button down the middle.
  final box = spanOfTypeNamed('Stack', under: '_MovableWindow');
  if (box == null) return screen;
  final margin = box.inflate(56);
  return (tall
          ? Rect.fromLTRB(margin.left, 0, margin.right, screen.bottom)
          : margin)
      .intersect(screen);
}
