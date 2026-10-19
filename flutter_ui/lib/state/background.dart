// Lumit kept running with no window, so the phones still see this computer.
//
// With Keep running in the background on, closing the window only hides it.
// Lumit sits in the tray (the menu bar on macOS) with Open Lumit and Quit Lumit,
// and opening Lumit again shows the window instead of starting a second one.
// The runner does the hiding, the tray and the handing over
// (windows/runner/flutter_window.cpp, macos/Runner/MainFlutterWindow.swift,
// linux/runner/my_application.cc). This file says what Settings asked for and
// answers the runner.

import 'dart:ui' show AppExitType;

import 'package:flutter/services.dart';

import '../l10n/strings.dart';
import 'workspace.dart';

const MethodChannel _channel = MethodChannel('lumit/window');

/// Tell the runner whether closing the window keeps Lumit running, and
/// whether Lumit starts in the tray at sign-in. The runner writes or removes
/// that entry each time, so it follows the copy of Lumit that last started.
/// Never throws, a runner that isn't there is a test.
Future<void> applyBackground(Workspace workspace) async {
  try {
    await _channel.invokeMethod<void>('background', {
      'keep': workspace.keepRunning,
      'signIn': workspace.keepRunning && workspace.openAtSignIn,
      'open': l10n.trayOpen,
      'quit': l10n.trayQuit,
    });
  } catch (_) {
    // No runner listening.
  }
}

/// Bring the window back, for a question that needs one to be asked in.
Future<void> showWindow() async {
  try {
    await _channel.invokeMethod<void>('show');
  } catch (_) {
    // No runner listening.
  }
}

/// Answer the runner. [hidden] runs when the window has been closed into the
/// tray. Quit in the tray is an ordinary quit, so the unsaved work question
/// is asked the way closing the window asks it.
void listenToRunner({required VoidCallback hidden}) {
  _channel.setMethodCallHandler((call) async {
    switch (call.method) {
      case 'quit':
        await ServicesBinding.instance
            .exitApplication(AppExitType.cancelable);
      case 'hidden':
        hidden();
    }
  });
}
