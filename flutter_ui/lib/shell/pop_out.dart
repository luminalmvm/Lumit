// A panel in a window of its own, which can sit on another monitor.
//
// Flutter's windows are still behind a flag on the stable release, so this
// file is the only one that reaches into the framework for them. The pin is
// one exact version (CI's flutter-version), and Flutter may change these names
// in any release, so a version bump starts here: if this file still builds and
// a popped Viewer still opens, the rest has nothing to do with it.
//
// ignore_for_file: implementation_imports, invalid_use_of_internal_member

import 'package:flutter/foundation.dart' show kIsWeb;
import 'package:flutter/src/foundation/_features.dart' show isWindowingEnabled;
import 'package:flutter/src/widgets/_window.dart';
import 'package:flutter/widgets.dart';

/// Turn Flutter's windows on. Called first thing in `main`, because the
/// binding makes its window owner as it starts and reads the flag then.
/// Not in a browser, which has one page and no second window to give.
void enablePopOut() {
  if (!kIsWeb) isWindowingEnabled = true;
}

/// Whether a panel can be popped out here. False under `flutter test`, which
/// never runs `main`, so widget tests see the menu as it always was.
bool get popOutAvailable => isWindowingEnabled;

/// One popped-out window. [onClosed] is called once, when the window is
/// closed from its title bar or by the operating system.
class PopOutWindow {
  final RegularWindowController _controller;

  PopOutWindow._(this._controller);

  /// Open a window of [size] logical pixels. Null when the platform
  /// refuses, in which case the panel stays where it was.
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
}

class _Delegate with RegularWindowControllerDelegate {
  final VoidCallback onClosed;
  bool _told = false;

  _Delegate(this.onClosed);

  // The close button docks the panel rather than destroying the window here,
  // so the view leaves the tree before its window goes.
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
