// A nightly build wearing the regular icon.
//
// # In plain terms
//
// A nightly has its own icon, so nobody mistakes it for a release. Some people
// would rather keep the one they know, so a nightly carries the regular icon as
// well and can show it instead: in the Dock on macOS, and on the window, the
// taskbar and the Start menu on Windows. The runner does the showing (macos/
// Runner/MainFlutterWindow.swift, windows/runner/flutter_window.cpp); this
// file only says which file to show.
//
// On macOS it lasts as long as Lumit is running, and the Finder draws the icon
// the application was built with. On Windows the shortcuts in the Start menu
// and on the taskbar are given the same picture, so they keep it.
//
// Windows is also slow to notice that an update changed the icon: a pinned
// shortcut goes on drawing the old one. Every build says which icon is its
// own at each launch, and the runner passes that on to the shortcuts.
//
// Linux is left alone: a Wayland desktop takes the icon from the desktop entry
// and has no window icon to set.

import 'dart:io';

import 'package:flutter/services.dart';

const MethodChannel _channel = MethodChannel('lumit/app_icon');

/// Whether this platform's icon can be changed while Lumit runs.
bool get appIconCanChange => Platform.isMacOS || Platform.isWindows;

/// Show the regular icon, or go back to the one this build came with.
///
/// The regular icon is only inside a nightly build (scripts/stamp-nightly.sh
/// puts it there), so only one may ask for it. Never throws: an icon that
/// stays as it was is not worth a sentence anywhere.
Future<void> showRegularAppIcon(bool regular) async {
  if (!appIconCanChange) return;
  final beside = File(Platform.resolvedExecutable).parent.path;
  final assets = Platform.isMacOS
      ? '$beside/../Frameworks/App.framework/Resources/flutter_assets'
      : '$beside\\data\\flutter_assets';
  final icon = Platform.isMacOS ? 'stable-icon.png' : 'stable-icon.ico';
  try {
    await _channel.invokeMethod<void>(
        'set', regular ? '$assets/assets/brand/$icon' : null);
  } catch (_) {
    // No runner listening, which is a test.
  }
}
