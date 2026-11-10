// The native window behind a floating window's controller, for the runner.
// Apart from pop_out.dart, the one place that reaches into the framework's
// windows, since each platform's controller says it in its own type.
//
// ignore_for_file: implementation_imports, invalid_use_of_internal_member

import 'package:flutter/src/widgets/_window_linux.dart'
    show WindowControllerLinux;
import 'package:flutter/src/widgets/_window_macos.dart'
    show WindowControllerMacOS;
import 'package:flutter/src/widgets/_window_win32.dart'
    show WindowControllerWin32;

/// An HWND, an NSWindow or a GtkWindow, as a number. Zero once it has gone.
int nativeHandleOf(Object controller) {
  try {
    return switch (controller) {
      WindowControllerWin32 c => c.windowHandle.address,
      WindowControllerMacOS c => c.windowHandle.address,
      WindowControllerLinux c => c.windowHandle.address,
      _ => 0,
    };
  } catch (_) {
    return 0;
  }
}
