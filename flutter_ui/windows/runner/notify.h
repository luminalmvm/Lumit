#ifndef RUNNER_NOTIFY_H_
#define RUNNER_NOTIFY_H_

#include <windows.h>

#include <string>

// What a window is sent when a notification it showed is clicked.
constexpr UINT kNotifyClickMessage = WM_APP + 2;

// Show [title] and [body] as one of the system's notifications, in Lumit's
// name (lib/state/background.dart). A click sends [window]
// kNotifyClickMessage. One that can't be shown, with notifications turned off
// for instance, says nothing.
void ShowNotification(HWND window, const std::wstring& title,
                      const std::wstring& body);

#endif  // RUNNER_NOTIFY_H_
