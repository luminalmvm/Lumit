// Manual screenshots, sweep: the welcome screen (K-464, K-468).
//
// welcome.png — the screen a plain launch opens on: the three cards, the
// RECENT well with thumbnails, one row wearing the placeholder, and the
// footer. Staged with three remembered projects; two of them are given a
// saved thumbnail (painted here, since a thumbnail is only ever a picture of
// a comp somebody saved), the third deliberately has none so the placeholder
// is in the shot.
//
//   cargo build -p lumit_bridge
//   cd flutter_ui
//   $env:LUMIT_SHOTS=1   # PowerShell; LUMIT_SHOTS=1 elsewhere
//   flutter run -d windows -t tool/shots/shots_welcome.dart

import 'dart:async';
import 'dart:io';
import 'dart:typed_data';
import 'dart:ui' as ui;

import 'package:flutter/material.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/state/workspace.dart';

import 'shots_common.dart';

/// A believable little save-time preview: a soft two-stop gradient in the
/// given hues, 128×72 like the real capture writes.
Future<Uint8List> _paintedThumb(Color a, Color b) async {
  final rec = ui.PictureRecorder();
  final canvas = Canvas(rec);
  const size = Size(128, 72);
  canvas.drawRect(
    Offset.zero & size,
    Paint()
      ..shader = ui.Gradient.linear(
          Offset.zero, Offset(size.width, size.height), <Color>[a, b]),
  );
  final img = await rec.endRecording().toImage(128, 72);
  final bytes = await img.toByteData(format: ui.ImageByteFormat.png);
  return bytes!.buffer.asUint8List();
}

Future<void> main() async {
  final (state, ui2) = await bootLumit();

  // Three recents, newest first once remembered in this order.
  const older = 'C:/Users/edit/Videos/Client sting/Client sting.lum';
  const yesterday = 'C:/Users/edit/Videos/Lower thirds/Lower thirds.lum';
  const today = 'C:/Users/edit/Videos/Opening titles/Opening titles.lum';
  ui2.workspace.rememberProject(older);
  ui2.workspace.rememberProject(yesterday);
  ui2.workspace.rememberProject(today);

  Workspace.writeThumbnail(today,
      await _paintedThumb(const Color(0xff2d3f6b), const Color(0xff6b3f5c)));
  Workspace.writeThumbnail(yesterday,
      await _paintedThumb(const Color(0xff1d4a44), const Color(0xff33406b)));
  // `older` gets no file, so its row wears the placeholder.

  runApp(shotRoot(LumitAppNew(state, ui2, welcome: true)));

  await pause(2);
  await sizeWindow(1280, 800);
  // The splash hands over to the welcome on its own; give it room to.
  await pause(4);
  await captureUi('welcome.png');
  exit(0);
}
