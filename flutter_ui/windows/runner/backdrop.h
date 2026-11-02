#ifndef RUNNER_BACKDROP_H_
#define RUNNER_BACKDROP_H_

#include <windows.h>

#include <string>

// A see-through window, when the style asks for one
// (lib/state/window_backdrop.dart). "tint" is the desktop picture's colour,
// soft and still: Mica. "blur" is whatever is behind the window, blurred:
// Acrylic. Windows 10 has neither and blurs behind the window for both.
// Anything else puts the window back to solid. [dark] picks the dark or light
// version of the material, to match the colour scheme.
//
// Answers whether the window is see-through now. It isn't when Transparency
// effects is off in Windows' settings, and Flutter keeps its room solid then.
bool SetBackdrop(HWND window, const std::string& kind, bool dark);

#endif  // RUNNER_BACKDROP_H_
