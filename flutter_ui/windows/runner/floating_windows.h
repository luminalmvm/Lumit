#ifndef RUNNER_FLOATING_WINDOWS_H_
#define RUNNER_FLOATING_WINDOWS_H_

#include <flutter/encodable_value.h>
#include <windows.h>

#include <cstdint>
#include <functional>

// Lumit's floating windows: the engine's own top-level windows
// (FLUTTER_HOST_WINDOW) that hold panels taken out of the main window
// (lib/shell/pop_out.dart). The engine makes and shows them; this makes them
// Lumit's. Each is owned by the main window, so it stays above it, goes with
// it when it is minimised and has no taskbar button of its own. Where Lumit
// draws the title line it has no system one, as the main window has none
// (win32_window.cpp). Every move is told back, so Dart can remember it.
//
// Everything here runs on the window's own thread, which is Flutter's.
namespace floating {

// Says |method| to Dart about |window|.
using Tell = std::function<void(const char* method, HWND window)>;

// Whether |window| is one of the engine's windows, which is what every
// floating window is.
bool IsFloating(HWND window);

// Whether there is a floating window open, shown or not.
bool Any();

// Make |window| one of Lumit's: owned by |main|, without the system's title
// bar when |own_title|, above other programs' windows when |on_top|, and
// watched for moves. A |quiet| one gives the keyboard back to |main| once it
// first shows: a window reopened at start is not one the user just made.
void Adopt(HWND window, HWND main, bool own_title, bool on_top, bool quiet,
           Tell tell);

// Where |window| is, as lib/shell/pop_out.dart's NativeFrame reads it.
flutter::EncodableMap Frame(HWND window);

// Put |window| at |bounds|, in screen coordinates, as it stands when it is
// neither maximised nor full screen, and maximise it or not.
void SetFrame(HWND window, const RECT& bounds, bool maximised);

// Keep |window| above other programs' windows, or not.
void SetOnTop(HWND window, bool on);

// Make |window| a video monitor: no frame, filling |display|, never taking
// the keyboard.
void Monitor(HWND window, const RECT& display);

// Where |window|'s maximise button is, in its client area's physical pixels.
void SetMaximiseButton(HWND window, const RECT& button);

// Every display, the primary one first, as NativeDisplay reads them.
flutter::EncodableList Displays();

// The outline of the window a dragged panel would make, at |rect| in
// |argb|, while the drag is over none of Lumit's windows. It never takes the
// pointer or the keyboard. Hidden again with HideGhost.
void ShowGhost(const RECT& rect, uint32_t argb);
void HideGhost();

// Hide the floating windows with the main window when it goes to the tray,
// and show them again when it comes back.
void HideAll();
void ShowHidden();

}  // namespace floating

#endif  // RUNNER_FLOATING_WINDOWS_H_
