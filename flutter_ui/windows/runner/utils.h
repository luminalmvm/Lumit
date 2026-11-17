#ifndef RUNNER_UTILS_H_
#define RUNNER_UTILS_H_

#include <windows.h>

#include <string>
#include <vector>

// Creates a console for the process, and redirects stdout and stderr to
// it for both the runner and the Flutter library.
void CreateAndAttachConsole();

// Takes a null-terminated wchar_t* encoded in UTF-16 and returns a std::string
// encoded in UTF-8. Returns an empty std::string on failure.
std::string Utf8FromUtf16(const wchar_t* utf16_string);

// Gets the command line arguments passed in as a std::vector<std::string>,
// encoded in UTF-8. Returns an empty std::vector<std::string> on failure.
std::vector<std::string> GetCommandLineArguments();

// How far a frameless window's frame reaches past what is seen, in physical
// pixels. A maximised window hangs over the screen by this much on every side.
int FrameThickness(HWND window);

// Whether a point on the screen, as WM_NCHITTEST carries it, is on the top
// resize edge of a window whose title bar Lumit draws. The title bar used to
// hold that edge. Now it is the first few pixels of the client area, half the
// frame's thickness so what is under it keeps most of its height. A
// maximised window has no edge to drag.
bool OnTopEdge(HWND window, LPARAM point);

#endif  // RUNNER_UTILS_H_
