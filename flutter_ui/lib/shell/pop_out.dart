// Panels in windows of their own, which can sit on other monitors, and the
// questions about windows and displays that only the runner can answer.
//
// Flutter's windows are still behind a flag on the stable release, so this
// file and pop_out_handle.dart are the only ones that reach into the
// framework for them. The pin is
// one exact version (CI's flutter-version), and Flutter may change these names
// in any release, so a version bump starts here: if this file still builds and
// a floating window still opens, the rest has nothing to do with it.
//
// Where a window stands, which displays there are and how a window is drawn
// are the runner's to say (windows/runner/flutter_window.cpp,
// macos/Runner/MainFlutterWindow.swift, linux/runner/my_application.cc), over
// the `lumit/window` channel the main window's buttons already use. A floating
// window is named to it by its native handle. Positions are in the units the
// system places windows in: physical pixels on Windows, points on macOS, and
// GDK's own units on Linux. `scale` says how many of those make one of
// Flutter's logical pixels in that window.
//
// ignore_for_file: implementation_imports, invalid_use_of_internal_member

import 'dart:math' show min;

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/services.dart';
import 'package:flutter/src/foundation/_features.dart' show isWindowingEnabled;
import 'package:flutter/src/widgets/_window.dart';
import 'package:flutter/widgets.dart';

import '../state/background.dart' show onWindowEvent;
import '../state/dock.dart' show FloatingFrame;
import 'pop_out_handle.dart'
    if (dart.library.js_interop) 'pop_out_handle_web.dart';

/// Turn Flutter's windows on. Called first thing in `main`, because the
/// binding makes its window owner as it starts and reads the flag then.
/// Not in a browser, which has one page and no second window to give.
void enablePopOut() {
  if (kIsWeb) return;
  isWindowingEnabled = true;
  onWindowEvent = NativeWindows.heard;
}

/// Whether a panel can be popped out here. False under `flutter test`, which
/// never runs `main`, so widget tests see the menu as it always was.
bool get popOutAvailable => isWindowingEnabled;

/// Lumit is on its way out: a floating window closing now keeps its panels,
/// so the arrangement opens the same way next time.
bool windowsQuitting = false;

/// One floating window. [onClosed] is called once, when the window is
/// closed from its title bar or by the operating system.
class PopOutWindow {
  final RegularWindowController _controller;

  PopOutWindow._(this._controller);

  /// Open a window of [size] logical pixels. Null when the platform
  /// refuses, in which case the panels stay where they were.
  static PopOutWindow? open({
    required String title,
    required Size size,
    required VoidCallback onClosed,
  }) {
    try {
      return PopOutWindow._(RegularWindowController(
        size: size,
        title: title,
        delegate: _Delegate(onClosed),
      ));
    } catch (error, stack) {
      FlutterError.reportError(FlutterErrorDetails(
          exception: error, stack: stack, library: 'pop out'));
      return null;
    }
  }

  /// The window's own view, with [child] drawn in it. Goes in a
  /// [ViewAnchor], so the child sees everything its place in the tree does.
  Widget view(Widget child) =>
      RegularWindow(controller: _controller, child: child);

  /// Close the window. Safe to call more than once.
  void close() => _controller.destroy();

  /// Told when the window is resized, put in front or sent back, and on
  /// Linux when it moves.
  Listenable get changes => _controller;

  /// The view Flutter draws the window in.
  int get viewId => _controller.rootView.viewId;

  /// The native window as the runner knows it: an HWND, an NSWindow or a
  /// GtkWindow. Zero once it has gone.
  int get handle => nativeHandleOf(_controller);

  bool get isActivated => _ask(() => _controller.isActivated);
  bool get isMaximised => _ask(() => _controller.isMaximized);
  bool get isMinimised => _ask(() => _controller.isMinimized);
  bool get isFullScreen => _ask(() => _controller.isFullscreen);

  void activate() => _do(_controller.activate);
  void setMaximised(bool on) => _do(() => _controller.setMaximized(on));
  void setMinimised(bool on) => _do(() => _controller.setMinimized(on));
  void setFullScreen(bool on) => _do(() => _controller.setFullscreen(on));
  void setTitle(String title) => _do(() => _controller.setTitle(title));

  // A window the system has already taken down answers no questions.
  static bool _ask(bool Function() question) {
    try {
      return question();
    } catch (_) {
      return false;
    }
  }

  static void _do(VoidCallback act) {
    try {
      act();
    } catch (_) {
      // Gone.
    }
  }
}

class _Delegate with RegularWindowControllerDelegate {
  final VoidCallback onClosed;
  bool _told = false;

  _Delegate(this.onClosed);

  // The close button puts the panels back rather than destroying the window
  // here, so the view leaves the tree before its window goes.
  @override
  void onWindowCloseRequested(RegularWindowController controller) => _tell();

  @override
  void onWindowDestroyed() => _tell();

  void _tell() {
    if (_told) return;
    _told = true;
    onClosed();
  }
}

// --- What the runner says about windows and displays ----------------------

const MethodChannel _channel = MethodChannel('lumit/window');

/// Where a window stands, as the runner reports it.
///
/// [bounds] is the window as it is when neither maximised nor full screen.
/// [client] is where Flutter draws in it now, which is how a pointer in one
/// window is found in another. [display] and [work] are the display the
/// window is mostly on and the part of it not taken by the system's own bars.
typedef NativeFrame = ({
  Rect bounds,
  bool maximised,
  bool fullScreen,
  bool minimised,
  Rect client,
  double scale,
  Rect? display,
  Rect? work,
});

/// One display, as the runner reports it.
typedef NativeDisplay = ({
  String name,
  Rect bounds,
  Rect work,
  double scale,
  bool primary,
});

/// Calls into the runner about windows. Each answers null, or does nothing,
/// where no runner listens: a widget test, or a browser.
abstract final class NativeWindows {
  /// Told when a window the runner watches has stopped moving, by its handle.
  static void Function(int window)? moved;

  /// Told when a display comes, goes or changes.
  static VoidCallback? displaysChanged;

  /// What the runner says on the channel about windows, from the one handler
  /// the channel has (state/background.dart). Says whether it was one of
  /// these.
  static bool heard(String method, Object? arguments) {
    switch (method) {
      case 'windowMoved':
        final window = arguments is Map ? arguments['window'] : null;
        if (window is int) moved?.call(window);
        return true;
      case 'displaysChanged':
        displaysChanged?.call();
        return true;
    }
    return false;
  }

  static Future<T?> _call<T>(String method, [Object? arguments]) async {
    if (kIsWeb) return null;
    try {
      return await _channel.invokeMethod<T>(method, arguments);
    } catch (_) {
      return null;
    }
  }

  /// Make [window] one of Lumit's floating windows: kept above the main
  /// window and minimised with it, drawn with Lumit's own title line when
  /// [ownTitle] is set, and watched, so a move is told back as `windowMoved`.
  /// A [quiet] one leaves the keyboard with the main window as it comes up.
  static Future<void> adopt(int window,
          {required bool ownTitle, required bool onTop, bool quiet = false}) =>
      _call<void>('adopt', {
        'window': window,
        'title': ownTitle,
        'onTop': onTop,
        'quiet': quiet,
      });

  /// Where [window] is, or the main window with none.
  static Future<NativeFrame?> frame([int? window]) async {
    final raw = await _call<Map<Object?, Object?>>(
        'frame', {if (window != null) 'window': window});
    if (raw == null) return null;
    final bounds = rectFrom(raw['bounds']);
    final client = rectFrom(raw['client']);
    if (bounds == null || client == null) return null;
    final scale = raw['scale'];
    return (
      bounds: bounds,
      maximised: raw['maximised'] == true,
      fullScreen: raw['fullScreen'] == true,
      minimised: raw['minimised'] == true,
      client: client,
      scale: scale is num && scale > 0 ? scale.toDouble() : 1.0,
      display: rectFrom(raw['display']),
      work: rectFrom(raw['work']),
    );
  }

  /// Put [window] at [bounds] and maximise it or not. Full screen is the
  /// controller's to set.
  static Future<void> setFrame(int window, Rect bounds,
          {bool maximised = false}) =>
      _call<void>('setFrame', {
        'window': window,
        'bounds': [bounds.left, bounds.top, bounds.width, bounds.height],
        'maximised': maximised,
      });

  /// Keep [window] above every other program's windows, or not.
  static Future<void> setOnTop(int window, bool on) =>
      _call<void>('onTop', {'window': window, 'on': on});

  /// Make [window] a video monitor: no frame, filling [display], and never
  /// taking the keyboard as it comes up.
  static Future<void> monitor(int window, Rect display) =>
      _call<void>('monitor', {
        'window': window,
        'display': [display.left, display.top, display.width, display.height],
      });

  /// Show the outline of the window a dragged panel would make, at [rect] in
  /// [colour], over everything and taking nothing. Null takes it away.
  static Future<void> ghost(Rect? rect, [Color? colour]) =>
      _call<void>('ghost', {
        if (rect != null && colour != null) ...{
          'rect': [rect.left, rect.top, rect.width, rect.height],
          'colour': colour.toARGB32(),
        },
      });

  /// Every display, the primary one first.
  static Future<List<NativeDisplay>> displays() async {
    final raw = await _call<List<Object?>>('displays');
    if (raw == null) return const [];
    final out = <NativeDisplay>[];
    for (final d in raw) {
      if (d is! Map) continue;
      final bounds = rectFrom(d['bounds']);
      if (bounds == null) continue;
      final scale = d['scale'];
      out.add((
        name: d['name'] is String ? d['name'] as String : '',
        bounds: bounds,
        work: rectFrom(d['work']) ?? bounds,
        scale: scale is num && scale > 0 ? scale.toDouble() : 1.0,
        primary: d['primary'] == true,
      ));
    }
    out.sort((a, b) => a.primary == b.primary ? 0 : (a.primary ? -1 : 1));
    return out;
  }

  /// A rectangle the runner sent as `[x, y, width, height]`.
  static Rect? rectFrom(Object? raw) {
    if (raw is! List || raw.length != 4 || raw.any((n) => n is! num)) {
      return null;
    }
    final r = Rect.fromLTWH((raw[0] as num).toDouble(),
        (raw[1] as num).toDouble(), (raw[2] as num).toDouble(),
        (raw[3] as num).toDouble());
    return r.isFinite && r.width > 0 && r.height > 0 ? r : null;
  }
}

// --- Where a window goes --------------------------------------------------

/// Which displays there are, as one string an arrangement is remembered
/// against: each display's place and size, in order. Empty with none known.
String displaySetupOf(List<NativeDisplay> displays) {
  final each = [
    for (final d in displays)
      '${d.bounds.left.round()},${d.bounds.top.round()},'
          '${d.bounds.width.round()}x${d.bounds.height.round()}',
  ]..sort();
  return each.join(' ');
}

/// Where a floating window should open, given where it was last ([frame]),
/// the displays there are now and where the main window is.
///
/// A window on a display that is still there, with enough of its top on
/// screen to take hold of, opens where it was, no bigger than the display.
/// One whose display has gone, or that would open out of reach, comes over
/// the main window at the size it had, each one [cascade] steps along from
/// the last. Full screen is kept only on the display it was full screen on.
/// A window never placed is [size] logical pixels plus its title line, over
/// the main window. With no displays to go by, [frame] is kept as it is.
FloatingFrame? fitWindowFrame(
  FloatingFrame? frame, {
  required List<NativeDisplay> displays,
  NativeFrame? main,
  Size? size,
  int cascade = 0,
}) {
  if (displays.isEmpty) return frame;
  final home = _displayAt(displays, main?.bounds) ?? displays.first;
  if (frame != null) {
    final on = _displayAt(displays, frame.bounds);
    if (on != null && _withinReach(frame.bounds, on.work)) {
      final full = frame.fullScreen &&
          displays.any((d) => d.bounds == frame.display);
      final bounds = Rect.fromLTWH(
          frame.bounds.left,
          frame.bounds.top,
          min(frame.bounds.width, on.work.width),
          min(frame.bounds.height, on.work.height));
      return FloatingFrame(bounds,
          maximised: frame.maximised, fullScreen: full, display: on.bounds);
    }
    return _over(home, main, frame.bounds.size, cascade,
        maximised: frame.maximised);
  }
  final units = main?.scale ?? home.scale;
  final want = size ?? const Size(960, 600);
  return _over(home, main,
      Size(want.width * units, (want.height + 32) * units), cascade);
}

/// Centred over the main window on [display], cascaded, inside its work area.
FloatingFrame _over(NativeDisplay display, NativeFrame? main, Size size,
    int cascade, {bool maximised = false}) {
  final work = display.work;
  final width = min(size.width, work.width * 0.9);
  final height = min(size.height, work.height * 0.9);
  final centre = main != null && !main.minimised && !main.maximised
      ? main.bounds.center
      : work.center;
  final step = 30.0 * (main?.scale ?? display.scale) * cascade;
  var rect = Rect.fromCenter(
      center: centre + Offset(step, step), width: width, height: height);
  final dx = rect.left < work.left
      ? work.left - rect.left
      : rect.right > work.right
          ? work.right - rect.right
          : 0.0;
  final dy = rect.top < work.top
      ? work.top - rect.top
      : rect.bottom > work.bottom
          ? work.bottom - rect.bottom
          : 0.0;
  rect = rect.shift(Offset(dx, dy));
  return FloatingFrame(rect, maximised: maximised, display: display.bounds);
}

/// The display [rect] is mostly on, or null when it is on none.
NativeDisplay? _displayAt(List<NativeDisplay> displays, Rect? rect) {
  if (rect == null) return null;
  NativeDisplay? best;
  var most = 0.0;
  for (final d in displays) {
    final shared = d.bounds.intersect(rect);
    final area = shared.width <= 0 || shared.height <= 0
        ? 0.0
        : shared.width * shared.height;
    if (area > most) {
      most = area;
      best = d;
    }
  }
  return best;
}

/// Whether enough of a window's top is in [work] to take hold of and move.
bool _withinReach(Rect window, Rect work) {
  final top = Rect.fromLTWH(
      window.left, window.top, window.width, min(window.height, 40));
  final shared = top.intersect(work);
  return shared.width >= 96 && shared.height >= 12;
}
