// Whether Alt is *really* held, asked of the operating system.
//
// In plain terms: Alt is Windows' menu-activation chord, so the key-up that
// ends an Alt+wheel gesture is often swallowed before the app sees it. Flutter
// then believes Alt is still down — and `syncKeyboardState` cannot put that
// right, because it re-asks the same embedding that missed the key-up. The only
// honest witness is the OS itself (K-334).
//
// **`GetAsyncKeyState`, not `GetKeyState`.** The first fix asked `GetKeyState`,
// which reads the keyboard state of the *calling thread's* message queue — and
// Dart's UI thread is not the Win32 thread that receives keyboard messages, so
// the answer was as stale as the one it was meant to correct. `GetAsyncKeyState`
// reads the physical key state, whoever asks (K-335).
//
// Only ever corrects a FALSE POSITIVE: when the framework already says Alt is
// up, that answer stands without a call. Under `flutter test` there is no real
// keyboard, so simulated modifiers are trusted as sent.

import 'dart:ffi';
import 'dart:io';

import 'package:flutter/services.dart';

const int _vkMenu = 0x12;

typedef _GetAsyncKeyStateC = Int16 Function(Int32);
typedef _GetAsyncKeyStateD = int Function(int);

_GetAsyncKeyStateD? _getAsyncKeyState;

bool get _canAskOs =>
    Platform.isWindows && !Platform.environment.containsKey('FLUTTER_TEST');

/// True when Alt is held according to the framework AND — on a real Windows
/// session — the key is physically down right now.
bool altActuallyHeld() {
  if (!HardwareKeyboard.instance.isAltPressed) return false;
  if (!_canAskOs) return true;
  _getAsyncKeyState ??= DynamicLibrary.open('user32.dll')
      .lookupFunction<_GetAsyncKeyStateC, _GetAsyncKeyStateD>(
          'GetAsyncKeyState');
  return (_getAsyncKeyState!(_vkMenu) & 0x8000) != 0;
}
