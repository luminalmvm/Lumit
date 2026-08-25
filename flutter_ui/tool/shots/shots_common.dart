// Shared plumbing for the manual-screenshot sweeps in this folder.
//
// **Why these are app entrypoints and not integration tests.** The obvious
// place for this was `integration_test/`, driven by
// `flutter test integration_test/… -d windows`. That harness does open the real
// runner window, and the engine really renders into it — `shared_texture_test`
// still reports the shared texture composited twenty-nine times — but on
// Flutter 3.44 nothing the test pumps ever reaches the screen: the window is
// plain white, whether it is showing the whole editor or a single orange box.
// A screenshot tool that photographs a white rectangle is no tool at all.
//
// So a sweep is the real application instead: the same startup `lib/main.dart`
// performs, the same `LumitAppNew` widget tree, the same engine — staged with a
// project, then photographed from outside and closed. What the manual shows is
// then the program itself rather than a harness impersonating it, which is the
// stronger position for a screenshot to be in anyway.
//
// Run one with:
//   $env:LUMIT_SHOTS=1
//   flutter run -d windows -t tool/shots/shots_1.dart
//
// Without `LUMIT_SHOTS=1` a sweep prints one line and quits, so nothing
// automatic can find itself driving the editor and writing PNGs into the site.
// The sweep exits by itself when the last shot is taken.

import 'dart:io';
import 'dart:ui' as ui;

import 'package:flutter/gestures.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:lumit_flutter/data/expressions_metadata.dart';
import 'package:lumit_flutter/main.dart';
import 'package:lumit_flutter/panels/effect_param_row_frb.dart';
import 'package:lumit_flutter/src/rust/frb_generated.dart';
import 'package:lumit_flutter/state/workspace.dart';
import 'package:lumit_flutter/theme/theme.dart';
import 'package:lumit_flutter/widgets/controls.dart';

/// Where the media fixtures live. Made once with ffmpeg (see the repo's shots
/// report) and never committed.
const fixtures = 'C:/tmp/lumit-shots';

/// The shape every sweep stages in — `LUMIT_SHOTS_SHAPE=round` picks Round
/// (K-394), which is what Settings ▸ Appearance ▸ Shape sets. The manual is
/// shot in the look it documents, so this is set once for a whole pass rather
/// than sweep by sweep.
final shotShape = ThemeShape.values
        .asNameMap()[Platform.environment['LUMIT_SHOTS_SHAPE'] ?? ''] ??
    ThemeShape.sharp;

/// How far above a docked panel's content its tab strip starts, in logical
/// pixels: the 26px strip itself, plus the pane card's padding under Round
/// (`dock_widget.dart`, `tokens.cardPadding` and its 1px boundary).
final dockTabInset = shotShape == ThemeShape.round ? 37.0 : 26.0;

/// How far outside a docked panel's content its pane card runs. Under Round the
/// rounded edge and its shadow are part of what a panel looks like, so a crop
/// taken at the content's own box cuts the design off; under Sharp the content
/// *is* the pane and there is nothing outside it.
final paneCardInset = shotShape == ThemeShape.round ? 13.0 : 0.0;

/// `flutter run` starts the built exe from `build/windows/x64/runner/Debug`,
/// not from `flutter_ui`, so every path here is worked out from the executable
/// rather than assumed. Six levels up is `flutter_ui`.
Directory get _flutterUi {
  var dir = File(Platform.resolvedExecutable).parent;
  for (var i = 0; i < 5; i++) {
    dir = dir.parent;
  }
  return dir;
}

/// Where a finished shot goes.
String get shotsOut =>
    Platform.environment['LUMIT_SHOTS_OUT'] ??
    '${_flutterUi.parent.path}/web-docs/src/assets/shots';

String get _script => '${_flutterUi.path}/tool/shots/capture_window.ps1';

/// Wall-clock seconds — the engine renders on its own threads, so the only way
/// to let a picture arrive is to wait for it.
Future<void> pause(double seconds) =>
    Future<void>.delayed(Duration(milliseconds: (seconds * 1000).round()));

/// Boot the application exactly as `lib/main.dart` does, but against a
/// throwaway settings file.
///
/// The throwaway store matters twice: the shots must show the factory
/// appearance and the factory panel arrangement rather than whatever the
/// machine's owner has set, and a sweep must not write over their settings on
/// its way past.
Future<(LumitState, LumitUiState)> bootLumit() async {
  // A sweep only runs when it is asked for by name. These are working tools —
  // the retake mechanism for the manual's screenshots — and nothing automatic
  // should ever find itself driving the editor and writing PNGs into the site.
  if (Platform.environment['LUMIT_SHOTS'] != '1') {
    // ignore: avoid_print
    print('SKIPPED: a screenshot sweep runs only with LUMIT_SHOTS=1 set.');
    exit(0);
  }
  WidgetsFlutterBinding.ensureInitialized();
  await BridgeLib.init();
  await ExpressionsMetadata.load();
  await ExpressionTextEditingController.initSyntaxHighlighting();
  Workspace.storeOverride =
      '${Directory.systemTemp.createTempSync('lumit-shots').path}/workspace.json';
  final state = LumitState()..newProject();
  final ui = LumitUiState(state);
  // A fresh store reads as a first run, and the first-run dialogue would sit
  // over every shot in the sweep.
  ui.workspace.skipFirstRun();
  ui.workspace.setShape(shotShape);
  return (state, ui);
}

/// The boundary [captureUi] photographs. Wrap the app in it: `runApp(shotRoot(…))`.
final shotRootKey = GlobalKey();

Widget shotRoot(Widget child) =>
    RepaintBoundary(key: shotRootKey, child: child);

/// Find a live widget by the `ValueKey<String>` its source gives it.
///
/// The panels key everything worth aiming at — twirls, switches, the blend
/// dropdown, the graph editor's lens buttons — so a sweep can work the real
/// interface instead of reaching into panel state that is deliberately
/// private. Returns null when the key is not on screen, which is a staging
/// mistake worth printing rather than crashing on.
Element? elementByKey(String key) {
  final wanted = ValueKey<String>(key);
  Element? found;
  void visit(Element el) {
    if (found != null) return;
    if (el.widget.key == wanted) {
      found = el;
      return;
    }
    el.visitChildren(visit);
  }

  WidgetsBinding.instance.rootElement?.visitChildren(visit);
  return found;
}

int _pointer = 0;

/// Click the middle of whatever carries [key].
///
/// A real pointer down/up through [GestureBinding], because this is the
/// application and not a widget test: there is no `WidgetTester` here, and
/// calling a panel's callbacks directly would photograph a state the program
/// cannot actually be put into.
Future<bool> tapKey(String key, {double settle = 0.6}) async {
  final el = elementByKey(key);
  final box = el?.renderObject;
  if (box is! RenderBox || !box.attached) {
    // ignore: avoid_print
    print('TAP MISS $key');
    return false;
  }
  await tapAt(box.localToGlobal(box.size.center(Offset.zero)), settle: settle);
  return true;
}

/// Double-click whatever carries [key] — how a Sequence layer's view is
/// opened, among other things.
Future<bool> doubleTapKey(String key, {double settle = 1}) async {
  final box = elementByKey(key)?.renderObject;
  if (box is! RenderBox || !box.attached) {
    // ignore: avoid_print
    print('TAP MISS $key');
    return false;
  }
  final at = box.localToGlobal(box.size.center(Offset.zero));
  await tapAt(at, settle: 0.08);
  await tapAt(at, settle: settle);
  return true;
}

/// Right-click whatever carries [key] — the second of the two routes a
/// toolbar group's flyout opens by (the other is a press and hold).
Future<bool> rightTapKey(String key, {double settle = 1}) async {
  final box = elementByKey(key)?.renderObject;
  if (box is! RenderBox || !box.attached) {
    // ignore: avoid_print
    print('TAP MISS $key');
    return false;
  }
  final at = box.localToGlobal(box.size.center(Offset.zero));
  final id = ++_pointer;
  GestureBinding.instance.handlePointerEvent(
      PointerDownEvent(pointer: id, position: at, buttons: kSecondaryButton));
  await pause(0.06);
  GestureBinding.instance
      .handlePointerEvent(PointerUpEvent(pointer: id, position: at));
  await pause(settle);
  return true;
}

/// Click a point in window coordinates.
Future<void> tapAt(Offset at, {double settle = 0.6}) async {
  final id = ++_pointer;
  GestureBinding.instance
      .handlePointerEvent(PointerDownEvent(pointer: id, position: at));
  await pause(0.06);
  GestureBinding.instance
      .handlePointerEvent(PointerUpEvent(pointer: id, position: at));
  await pause(settle);
}

/// Where the widget carrying [key] sits, in logical pixels — so a crop is
/// worked out from the interface rather than guessed and re-guessed a build at
/// a time. Null when it is not on screen: a staging mistake, printed rather
/// than thrown, so a sweep goes on and says which shot to fix.
Rect? boxOf(String key) {
  final render = elementByKey(key)?.renderObject;
  if (render is! RenderBox || !render.attached) {
    // ignore: avoid_print
    print('BOX MISS $key');
    return null;
  }
  return render.localToGlobal(Offset.zero) & render.size;
}

/// Where the first widget of [type] sits, for the panels that carry no key of
/// their own — the Viewer's picture area is worked out from its panel, and a
/// panel is a class rather than a `ValueKey`.
Rect? boxOfType(Type type) {
  Element? found;
  void visit(Element el) {
    if (found != null) return;
    if (el.widget.runtimeType == type) {
      found = el;
      return;
    }
    el.visitChildren(visit);
  }

  WidgetsBinding.instance.rootElement?.visitChildren(visit);
  final render = found?.renderObject;
  if (render is! RenderBox || !render.attached) {
    // ignore: avoid_print
    print('BOX MISS $type');
    return null;
  }
  return render.localToGlobal(Offset.zero) & render.size;
}

/// The popup currently open — a dropdown's list, a menu's page, a dialogue —
/// with a margin around it. Every one of them is a [FloatSurface], so the crop
/// follows the thing rather than a band of numbers that only fits the shape and
/// the row count it was measured against.
Rect? openPopup({double margin = 12}) =>
    boxOfType(FloatSurface)?.inflate(margin);

/// Photograph the application's own render tree.
///
/// Read straight out of Flutter rather than off the screen, which is the only
/// capture on this machine that returns the interface at all: everything
/// composited through DirectComposition — the editor's whole client area — is
/// absent from a GDI screen grab, and DXGI desktop duplication hands back no
/// frames.
/// [crop] narrows the shot to one panel or one row, in the same logical pixels
/// [boxOf] measures in — for the pages whose caption is about a part of the
/// interface rather than the whole editor.
///
/// [scale] rasterises at more than one image pixel per logical pixel. The
/// controls the manual has to show are not all big: the cache bar is two
/// logical pixels tall, and a detail crop of it at 1:1 is a hairline nobody
/// can see once the page has scaled the picture down.
Future<void> captureUi(String name, {Rect? crop, double scale = 1}) async {
  final boundary =
      shotRootKey.currentContext!.findRenderObject()! as RenderRepaintBoundary;
  final image = await boundary.toImage(pixelRatio: scale);
  final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
  final out = File('$shotsOut/$name')
    ..writeAsBytesSync(bytes!.buffer.asUint8List());
  // `LUMIT_SHOTS_NOCROP=1` keeps the whole window, so a first pass can be
  // looked at and the crops worked out from what is actually there.
  if (Platform.environment['LUMIT_SHOTS_NOCROP'] == '1') crop = null;
  String? cut;
  if (crop != null) {
    final box = Rect.fromLTRB(crop.left * scale, crop.top * scale,
        crop.right * scale, crop.bottom * scale);
    cut = '${box.width.round()}:${box.height.round()}'
        ':${box.left.round().clamp(0, 1 << 20)}'
        ':${box.top.round().clamp(0, 1 << 20)}';
    final tmp = '${out.path}.crop.png';
    final r = await Process.run(
        'ffmpeg', ['-y', '-i', out.path, '-vf', 'crop=$cut', tmp]);
    if (r.exitCode == 0) {
      File(tmp).renameSync(out.path);
    } else {
      // ignore: avoid_print
      print('CROP FAILED $name: ${r.stderr}');
    }
  }
  // ignore: avoid_print
  print('SHOT $name: ${image.width}x${image.height}'
      '${cut == null ? '' : ' cropped $cut'}');
}

/// Resize the application window, and wait for the relayout.
///
/// Done from outside because there is no Dart call for it: the runner opens at
/// 1280×720 and the manual wants a working window. Every sweep should start
/// with this.
Future<void> sizeWindow(int width, int height) async {
  await _runScript(
    ['-Width', '$width', '-Height', '$height'],
    '${Directory.systemTemp.path}/lumit-shot-sizing.png',
  );
  await pause(2);
}

/// Photograph the application **window**, chrome and all, from outside.
///
/// Kept for the shots [captureUi] cannot reach — anything drawn in a second
/// operating-system window rather than in Lumit's own tree. Be warned: on the
/// machine this was written on it returns a blank client area whichever `mode`
/// is asked for, because Flutter's content is composited through
/// DirectComposition and neither `PrintWindow` nor a GDI screen grab can see
/// it (DXGI desktop duplication returns no frames at all here). Check what it
/// produced before believing it.
Future<void> capture(String name, {String mode = 'print'}) =>
    _runScript(const [], '$shotsOut/$name', mode: mode);

Future<void> _runScript(
  List<String> extra,
  String out, {
  String mode = 'print',
}) async {
  final result = await Process.run('powershell', [
    '-NoProfile',
    '-ExecutionPolicy',
    'Bypass',
    '-File',
    _script,
    '-Title',
    'Lumit',
    '-Mode',
    mode,
    '-Out',
    out,
    ...extra,
  ]);
  // ignore: avoid_print
  print('WINDOW ${result.stdout.toString().trim()}'
      '${result.stderr.toString().trim()}');
}
