// Whether Alt is *really* held, asked of the operating system.
//
// In plain terms: Alt is Windows' menu-activation chord, so the key-up that
// ends an Alt+wheel gesture is often swallowed before the app sees it. Flutter
// then believes Alt is still down — and `syncKeyboardState` cannot put that
// right, because it re-asks the same embedding that missed the key-up. The only
// honest witness is the OS itself.
//
// **`GetAsyncKeyState`, not `GetKeyState`.** The first fix asked `GetKeyState`,
// which reads the keyboard state of the *calling thread's* message queue — and
// Dart's UI thread is not the Win32 thread that receives keyboard messages, so
// the answer was as stale as the one it was meant to correct. `GetAsyncKeyState`
// reads the physical key state, whoever asks.
//
// Only ever corrects a FALSE POSITIVE: when the framework already says Alt is
// up, that answer stands without a call. Under `flutter test` there is no real
// keyboard, so simulated modifiers are trusted as sent.

import 'package:flutter/services.dart';
import 'package:lumit_flutter/state/win32.dart'
    if (dart.library.js_interop) 'package:lumit_flutter/state/win32_web.dart';
import 'package:lumit_flutter/state/platform.dart'
    if (dart.library.js_interop) 'package:lumit_flutter/state/platform_web.dart';

bool get _canAskOs =>
    Platform.isWindows && !Platform.environment.containsKey('FLUTTER_TEST');

/// True when Alt is held according to the framework AND — on a real Windows
/// session — the key is physically down right now.
bool altActuallyHeld() {
  if (!HardwareKeyboard.instance.isAltPressed) return false;
  if (!_canAskOs) return true;
  return altPhysicallyDown();
}
