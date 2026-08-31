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
import 'package:lumit_flutter/panels/timeline_panel_frb.dart';
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
Rect? boxOfType(Type type) => boxOfTypeNamed('$type');

/// The same, given the class's *name* rather than the class — for the widgets a
/// sweep has to aim at that their library does not export. The window every
/// dialogue opens on is one: `showLumitModal` builds a private `_MovableWindow`
/// around whatever it was handed, and nothing outside `modal_window.dart` can
/// write that type down.
///
/// [under] narrows the search to the subtree of the first widget by that name,
/// which is how a common class becomes a specific thing: the modal window's own
/// box is the whole app window — it centres its content rather than sizing
/// itself to it — so the box a dialogue shot wants is the `Stack` inside it.
Rect? boxOfTypeNamed(String name, {String? under}) {
  Element? find(Element root, String wanted) {
    Element? found;
    void visit(Element el) {
      if (found != null) return;
      if (el.widget.runtimeType.toString() == wanted) {
        found = el;
        return;
      }
      el.visitChildren(visit);
    }

    root.visitChildren(visit);
    return found;
  }

  var root = WidgetsBinding.instance.rootElement;
  if (root != null && under != null) root = find(root, under);
  final found = root == null ? null : find(root, name);
  final render = found?.renderObject;
  if (render is! RenderBox || !render.attached) {
    // ignore: avoid_print
    print('BOX MISS ${under == null ? name : '$name under $under'}');
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

/// The full visual extent of a widget: the union of every attached [RenderBox]
/// in its subtree, in the same logical pixels [boxOf] measures in.
///
/// [boxOfType] answers with the widget's *own* render object, and a widget that
/// is not itself a render object has none — so `Element.renderObject` hands
/// back whichever descendant box the walk reaches first. That is often an inner
/// box: the Timeline panel's stops short of both the Export button and the
/// strip along its foot, and a menu surface's stops short of its own width.
/// Every crop that came out with a control sliced down the middle came out that
/// way for this reason.
///
/// Unioning the subtree measures what is drawn instead. It is the wider
/// answer, so it is used where a crop must *contain* a widget rather than sit
/// tight against it — a shot cut short is a defect, a shot with a few spare
/// pixels is not.
///
/// **Cut it to the window when the panel clips something bigger than itself.**
/// A canvas that pans keeps render boxes for the things panned out of view, at
/// coordinates outside the window, and this walk unions those too — so the node
/// graph measured this way reaches out to wherever its furthest node happens to
/// sit. `spanOfType(…)!.intersect(Offset.zero & frameSize)` is the whole answer
/// there: everything the panel draws, bounded by what the window shows.
/// [boxOfType] is not the fix — on that same panel it is an inner box that
/// stops short of the last node.
/// [under] narrows the search to the subtree of the first widget by that name,
/// exactly as it does for [boxOfTypeNamed].
Rect? spanOfTypeNamed(String name, {String? under}) {
  Element? find(Element root, String wanted) {
    Element? found;
    void visit(Element el) {
      if (found != null) return;
      if (el.widget.runtimeType.toString() == wanted) {
        found = el;
        return;
      }
      el.visitChildren(visit);
    }

    root.visitChildren(visit);
    return found;
  }

  var root = WidgetsBinding.instance.rootElement;
  if (root != null && under != null) root = find(root, under);
  final found = root == null ? null : find(root, name);
  if (found == null) {
    // ignore: avoid_print
    print('SPAN MISS ${under == null ? name : '$name under $under'}');
    return null;
  }
  return spanOfElement(found);
}

/// [spanOfTypeNamed] for a class the sweep can write down.
Rect? spanOfType(Type type) => spanOfTypeNamed('$type');

/// A **docked** panel's rectangle: everything it draws, bounded by the pane
/// that clips it.
///
/// Reach for this whenever a docked panel clips something bigger than itself —
/// a list that scrolls, a canvas that pans — because there [boxOfType] and
/// [spanOfType] fail in opposite directions and each has produced a wrong
/// picture. The box is an inner one that slices a control off the edge; the
/// span runs out to wherever the clipped content goes. The Effects & presets
/// panel is both at once, and photographed by its span it came out with the
/// Timeline underneath it in the frame.
///
/// A panel whose content simply fits is measured by [spanOfType] in the sweeps
/// that came first, and correctly — the Viewer and the two tree panels among
/// them. Nothing is wrong with those; this is the safer default for new ones.
///
/// The pane is `_PaneChrome`, private to `dock_widget.dart`, one per docked
/// panel: found by walking up from the panel rather than by name, so it is
/// *this* panel's pane and not the first one in the tree. A panel that is not
/// in the dock has none, and then the span stands on its own.
Rect? dockedPanelBox(Type type) {
  final el = elementOfTypeNamed('$type');
  if (el == null) {
    // ignore: avoid_print
    print('PANEL MISS $type');
    return null;
  }
  final span = spanOfElement(el);
  Rect? pane;
  el.visitAncestorElements((up) {
    if (up.widget.runtimeType.toString() != '_PaneChrome') return true;
    final render = up.renderObject;
    if (render is RenderBox && render.attached && render.hasSize) {
      pane = render.localToGlobal(Offset.zero) & render.size;
    }
    return false;
  });
  if (span == null || pane == null) return span;
  return span.intersect(pane!);
}

/// The first widget of [name] in the tree, as an [Element].
Element? elementOfTypeNamed(String name, {String? under}) {
  Element? find(Element root, String wanted) {
    Element? found;
    void visit(Element el) {
      if (found != null) return;
      if (el.widget.runtimeType.toString() == wanted) {
        found = el;
        return;
      }
      el.visitChildren(visit);
    }

    root.visitChildren(visit);
    return found;
  }

  var root = WidgetsBinding.instance.rootElement;
  if (root != null && under != null) root = find(root, under);
  return root == null ? null : find(root, name);
}

/// The union of every attached [RenderBox] in [root]'s subtree, [root]'s own
/// included.
Rect? spanOfElement(Element root) {
  Rect? all;
  void visit(Element el) {
    final render = el.renderObject;
    if (render is RenderBox && render.attached && render.hasSize) {
      final box = render.localToGlobal(Offset.zero) & render.size;
      all = all == null ? box : all!.expandToInclude(box);
    }
    el.visitChildren(visit);
  }

  visit(root);
  return all;
}

/// Every popup on screen at once, as one box.
///
/// A menu with a category opened off it is *two* [FloatSurface]s, and
/// [openPopup] answers with whichever the walk reaches first — so a crop aimed
/// at it cuts the submenu in half down the side. This unions them instead, and
/// keeps doing so however many the interface has open.
Rect? openPopupsBox({double margin = 12}) {
  Rect? all;
  void visit(Element el) {
    if (el.widget is FloatSurface) {
      final span = spanOfElement(el);
      if (span != null) all = all == null ? span : all!.expandToInclude(span);
    }
    el.visitChildren(visit);
  }

  WidgetsBinding.instance.rootElement?.visitChildren(visit);
  return all?.inflate(margin);
}

/// The whole Timeline panel, tab strip to foot, pane card and all.
///
/// Taken as [spanOfType] — the union of everything the panel draws — and then
/// widened again by three parts that have each been the panel's true edge at
/// some point: the foot strip ([LaneBottomBar]: easing buttons in Graph mode,
/// drawing tools in Layers mode), the ruler, and the zoom slider.
///
/// The proxies this replaces (`ruler.top - 28`, `zoomSlider.bottom + 8`) were
/// each right when they were written and each went stale: the panel grew a
/// header row between its tab strip and the ruler, and Graph mode grew its
/// easing bar. A union goes stale only if the panel loses a part, which shows
/// up as a smaller crop rather than a clipped one.
///
/// Clamped to the photographed boundary — the client area, not the window that
/// was asked for. A crop poking one pixel past the frame is an ffmpeg refusal
/// and a silent full-window shot.
Rect timelinePanelBox() {
  var b = spanOfType(TimelinePanelFrb)!;
  for (final part in [
    boxOfType(LaneBottomBar),
    boxOf('tl-ruler'),
    boxOf('tl-zoom-slider'),
  ]) {
    if (part != null) b = b.expandToInclude(part);
  }
  b = b.inflate(paneCardInset + 2);
  final frame = shotRootKey.currentContext!.size!;
  return Rect.fromLTRB(
    b.left.clamp(0, frame.width),
    b.top.clamp(0, frame.height),
    b.right.clamp(0, frame.width),
    b.bottom.clamp(0, frame.height),
  );
}

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
    // Kept inside the picture, because ffmpeg does not refuse a crop that runs
    // off the edge — it **slides the whole rectangle back inside and crops
    // there**, silently. A shot whose floor came from a row below the fold
    // then photographs a region hundreds of pixels above the one it asked for,
    // exits 0, and prints the numbers it wanted rather than the ones it used.
    // That is how `shape-layer.png` came to be a picture of the Viewer.
    //
    // Intersecting instead truncates: the shot loses the part that was never
    // on screen and keeps the part that was, which is a picture with a piece
    // missing rather than a picture of somewhere else. The line says so, so a
    // staging mistake is read off the log instead of spotted by eye.
    final frame = Rect.fromLTWH(0, 0, image.width.toDouble(),
        image.height.toDouble());
    final asked = Rect.fromLTRB(crop.left * scale, crop.top * scale,
        crop.right * scale, crop.bottom * scale);
    final box = asked.intersect(frame);
    if (box.isEmpty) {
      // Nothing of what was asked for is on screen. The whole window is a
      // worse picture than the one wanted but a better one than a slid crop,
      // and the line says which shot to go and stage properly.
      // ignore: avoid_print
      print('CROP OFF-PICTURE $name: wanted $asked of $frame — shot whole');
    } else {
      // Only a loss worth knowing about is said out loud. A panel's shadow
      // hangs a dozen pixels off the edge of the client area, so an exact
      // comparison fires on shots that are perfectly fine — and a warning that
      // cries wolf on every pass is one nobody reads on the pass that matters.
      const slack = 8;
      final lost = [
        box.left - asked.left,
        box.top - asked.top,
        asked.right - box.right,
        asked.bottom - box.bottom,
      ].reduce((a, b) => a > b ? a : b);
      if (lost > slack) {
        // ignore: avoid_print
        print('CROP CLAMPED $name: wanted $asked, took $box'
            ' — ${lost.round()}px of it is not on screen');
      }
      cut = '${box.width.round()}:${box.height.round()}'
          ':${box.left.round()}:${box.top.round()}';
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
