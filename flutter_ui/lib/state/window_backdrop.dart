// A see-through window, when the style asks for one with `--backdrop`.
//
// The runner lays the system's own material behind the window: Mica or
// Acrylic on Windows (a blur behind on Windows 10), vibrancy on macOS, and
// the compositor's blur on Linux where it has one. What is behind Lumit then
// shows wherever the room is clear. The system draws it, so it costs Flutter
// nothing a frame, and the Viewer and playback are never under it.
//
// The runner answers whether it is showing. It isn't when the system has
// transparency switched off, or has no way to blur, and the room stays solid
// then. Settings ▸ Reduce transparency asks for none at all.

import 'package:flutter/scheduler.dart' show SchedulerBinding;
import 'package:flutter/services.dart';

const MethodChannel _channel = MethodChannel('lumit/window');

/// What a style's `--backdrop` asks for.
enum Backdrop {
  /// A solid window.
  none,

  /// The desktop picture's colour, soft and still.
  tint,

  /// Whatever is behind the window, blurred.
  blur,
}

abstract final class WindowBackdrop {
  static Backdrop _asked = Backdrop.none;
  static bool _dark = true;

  /// Whether the window is see-through now.
  static bool showing = false;

  /// What the window's root paints over it while [showing]: the style's
  /// room colour, clear unless it tints it.
  static Color tint = const Color(0x00000000);

  /// Called when [showing] changes, to redraw the room.
  static VoidCallback? onChange;

  /// Ask for [kind], made dark or light to match the colour scheme. Only a
  /// change is sent.
  static void want(Backdrop kind, {required bool dark}) {
    if (kind == _asked && (dark == _dark || kind == Backdrop.none)) return;
    _asked = kind;
    _dark = dark;
    _send();
  }

  /// Ask again, when the system's settings changed under the window.
  static void refresh() => _send();

  static Future<void> _send() async {
    final kind = _asked;
    // Going solid, the solid room is on screen before the material goes, or
    // a frame with a clear room and nothing behind it shows black.
    if (kind == Backdrop.none) {
      await SchedulerBinding.instance.endOfFrame;
      await SchedulerBinding.instance.endOfFrame;
      if (kind != _asked) return;
    }
    bool now;
    try {
      now = await _channel.invokeMethod<bool>(
              'backdrop', {'kind': kind.name, 'dark': _dark}) ??
          false;
    } catch (_) {
      // No runner listening: a test.
      now = false;
    }
    // A later ask has gone out since; its answer is the one that counts.
    if (kind != _asked || now == showing) return;
    showing = now;
    onChange?.call();
  }
}
