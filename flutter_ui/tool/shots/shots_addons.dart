// Manual screenshots, sweep: the Addons page of Settings.
//
// settings-addons-empty.png is the page on a machine with nothing installed
// and the catalogue not yet asked for. settings-addons.png is the same page
// once the runtime and the five packs are in, installed through the page's own
// Install from file path from folders holding each addon.json with its files
// beside it, so the shot is of a real install and not a staged list.
//
// LUMIT_ADDON_FOLDERS names those folders, separated by the platform's path
// separator. Without it only the empty shot is taken.
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1   # PowerShell; LUMIT_SHOTS=1 elsewhere
//   flutter run -d windows -t tool/shots/shots_addons.dart

// `_context` is re-read from the live tree at every use.
// ignore_for_file: use_build_context_synchronously

import 'dart:async';
import 'dart:io';
import 'dart:typed_data';

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/shell/settings_window_frb.dart';
import 'package:lumit_flutter/src/rust/api/composition.dart';
import 'package:lumit_flutter/src/rust/lib.dart';
import 'package:lumit_flutter/src/rust/api/effect.dart';
import 'shots_common.dart';

/// A plain 1920 by 1080 comp at 25 fps, so the shell has something to show
/// behind the window. Every field is required by the constructor.
BridgeCompSettings _settings(String name) => BridgeCompSettings(
      name: name,
      width: 1920,
      height: 1080,
      fpsNum: 25,
      fpsDen: 1,
      duration: BridgeRational(num: 10, den: 1),
      background: F32Array4(Float32List.fromList([0, 0, 0, 1])),
      shutterAngle: 180,
      motionBlurSamples: 16,
    );

/// A context out of the live tree: the Project panel's Import button sits under
/// the Overlay the settings window needs.
BuildContext get _context => elementByKey('project-import')!;

/// The settings window with a margin, clipped to the screen.
Rect _window() {
  final root = shotRootKey.currentContext!.findRenderObject()! as RenderBox;
  final screen = Offset.zero & root.size;
  final box = spanOfTypeNamed('Stack', under: '_MovableWindow');
  if (box == null) return screen;
  return box.inflate(56).intersect(screen);
}

Future<void> main() async {
  final (state, ui) = await bootLumit();
  final project = state.project!;
  final comp =
      project.newComposition(name: 'Untitled', settings: _settings('Untitled'));
  ui.setSelectedComp(comp);

  runApp(shotRoot(LumitAppNew(state, ui, welcome: false)));
  await pause(2);
  await sizeWindow(1720, 1000);
  await pause(6);

  unawaited(showSettingsWindowFrb(_context, initialPage: SettingsPage.addons));
  await pause(3);
  await captureUi('settings-addons-empty.png', scale: 2, crop: _window());

  final folders = Platform.environment['LUMIT_ADDON_FOLDERS'];
  if (folders == null || folders.isEmpty) exit(0);
  for (final folder in folders.split(Platform.isWindows ? ';' : ':')) {
    if (folder.isEmpty) continue;
    await ui.addons.installFromFile('$folder${Platform.pathSeparator}addon.json');
    await pause(1);
  }
  await ui.addons.runtimeLoad();
  await pause(3);
  await captureUi('settings-addons.png', scale: 2, crop: _window());
  exit(0);
}
