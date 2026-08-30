#include "win32_window.h"

#include <dwmapi.h>
#include <flutter_windows.h>

#include <fstream>

#include "resource.h"

namespace {

/// Window attribute that enables dark mode window decorations.
///
/// Redefined in case the developer's machine has a Windows SDK older than
/// version 10.0.22000.0.
/// See: https://docs.microsoft.com/windows/win32/api/dwmapi/ne-dwmapi-dwmwindowattribute
#ifndef DWMWA_USE_IMMERSIVE_DARK_MODE
#define DWMWA_USE_IMMERSIVE_DARK_MODE 20
#endif

constexpr const wchar_t kWindowClassName[] = L"FLUTTER_RUNNER_WIN32_WINDOW";

/// Registry key for app theme preference.
///
/// A value of 0 indicates apps should use dark mode. A non-zero or missing
/// value indicates apps should use light mode.
constexpr const wchar_t kGetPreferredBrightnessRegKey[] =
  L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize";
constexpr const wchar_t kGetPreferredBrightnessRegValue[] = L"AppsUseLightTheme";

// The number of Win32Window objects that currently exist.
static int g_active_window_count = 0;

using EnableNonClientDpiScaling = BOOL __stdcall(HWND hwnd);

// Scale helper to convert logical scaler values to physical using passed in
// scale factor
int Scale(int source, double scale_factor) {
  return static_cast<int>(source * scale_factor);
}

// Dynamically loads the |EnableNonClientDpiScaling| from the User32 module.
// This API is only needed for PerMonitor V1 awareness mode.
void EnableFullDpiSupportIfAvailable(HWND hwnd) {
  HMODULE user32_module = LoadLibraryA("User32.dll");
  if (!user32_module) {
    return;
  }
  auto enable_non_client_dpi_scaling =
      reinterpret_cast<EnableNonClientDpiScaling*>(
          GetProcAddress(user32_module, "EnableNonClientDpiScaling"));
  if (enable_non_client_dpi_scaling != nullptr) {
    enable_non_client_dpi_scaling(hwnd);
  }
  FreeLibrary(user32_module);
}

// Where the window's size, position and maximised state are remembered between
// runs: one file beside the interface's own settings, in the folder
// `Workspace.storeFile()` writes `flutter-workspace.json` into
// (`%APPDATA%\lumit`). This is machine state, never project state — nothing
// about a window's geometry belongs in a `.lum`.
//
// The file is a `WINDOWPLACEMENT` struct written raw. That is not laziness for
// its own sake: it is a fixed-size record with no pointers in it, it carries
// its own size in its first field so a struct from another build is spotted
// rather than misread, and it is the one structure that already describes a
// maximised window *and* the rectangle it un-maximises to. Losing it costs a
// window opening maximised, which is where it opens anyway.
//
// The folder is created here rather than in the save alone, so both halves go
// through one function; on every run after the first that is one failed
// `CreateDirectory` call at startup. Empty when `%APPDATA%` is unset or absurd,
// in which case nothing is remembered and nothing is an error.
std::wstring WindowPlacementPath() {
  wchar_t app_data[MAX_PATH];
  DWORD length = GetEnvironmentVariable(L"APPDATA", app_data, MAX_PATH);
  if (length == 0 || length >= MAX_PATH) {
    return std::wstring();
  }
  std::wstring directory = std::wstring(app_data, length) + L"\\lumit";
  CreateDirectory(directory.c_str(), nullptr);
  return directory + L"\\window-placement.bin";
}

// The geometry the last run closed at, or false when there is nothing to
// restore: a first run, a file from a build whose struct was a different size,
// or a window remembered on a monitor that has since been unplugged. Each of
// those opens maximised on the primary monitor instead.
bool LoadWindowPlacement(WINDOWPLACEMENT* placement) {
  const std::wstring path = WindowPlacementPath();
  if (path.empty()) {
    return false;
  }
  WINDOWPLACEMENT saved{};
  std::ifstream file(path.c_str(), std::ios::binary);
  if (!file.read(reinterpret_cast<char*>(&saved), sizeof(saved)) ||
      saved.length != sizeof(saved)) {
    return false;
  }
  // MONITOR_DEFAULTTONULL is the whole of the vanished-monitor check: it
  // answers with nothing when the rectangle touches no monitor that exists.
  if (MonitorFromRect(&saved.rcNormalPosition, MONITOR_DEFAULTTONULL) ==
      nullptr) {
    return false;
  }
  *placement = saved;
  return true;
}

// Remember where this window is, for the next run. Best effort throughout: a
// machine that will not give up `%APPDATA%` or will not take the file simply
// opens maximised next time.
void SaveWindowPlacement(HWND window) {
  WINDOWPLACEMENT placement{};
  placement.length = sizeof(placement);
  if (!GetWindowPlacement(window, &placement)) {
    return;
  }
  // Closing a minimised window must not reopen it minimised. Come back the way
  // it would have come back off the taskbar — which is the one thing the flags
  // field is remembering for us.
  if (placement.showCmd == SW_SHOWMINIMIZED) {
    placement.showCmd = (placement.flags & WPF_RESTORETOMAXIMIZED)
                            ? SW_SHOWMAXIMIZED
                            : SW_SHOWNORMAL;
  }
  const std::wstring path = WindowPlacementPath();
  if (path.empty()) {
    return;
  }
  std::ofstream file(path.c_str(), std::ios::binary | std::ios::trunc);
  file.write(reinterpret_cast<const char*>(&placement), sizeof(placement));
}

}  // namespace

// Manages the Win32Window's window class registration.
class WindowClassRegistrar {
 public:
  ~WindowClassRegistrar() = default;

  // Returns the singleton registrar instance.
  static WindowClassRegistrar* GetInstance() {
    if (!instance_) {
      instance_ = new WindowClassRegistrar();
    }
    return instance_;
  }

  // Returns the name of the window class, registering the class if it hasn't
  // previously been registered.
  const wchar_t* GetWindowClass();

  // Unregisters the window class. Should only be called if there are no
  // instances of the window.
  void UnregisterWindowClass();

 private:
  WindowClassRegistrar() = default;

  static WindowClassRegistrar* instance_;

  bool class_registered_ = false;
};

WindowClassRegistrar* WindowClassRegistrar::instance_ = nullptr;

const wchar_t* WindowClassRegistrar::GetWindowClass() {
  if (!class_registered_) {
    WNDCLASS window_class{};
    window_class.hCursor = LoadCursor(nullptr, IDC_ARROW);
    window_class.lpszClassName = kWindowClassName;
    window_class.style = CS_HREDRAW | CS_VREDRAW;
    window_class.cbClsExtra = 0;
    window_class.cbWndExtra = 0;
    window_class.hInstance = GetModuleHandle(nullptr);
    window_class.hIcon =
        LoadIcon(window_class.hInstance, MAKEINTRESOURCE(IDI_APP_ICON));
    window_class.hbrBackground = 0;
    window_class.lpszMenuName = nullptr;
    window_class.lpfnWndProc = Win32Window::WndProc;
    RegisterClass(&window_class);
    class_registered_ = true;
  }
  return kWindowClassName;
}

void WindowClassRegistrar::UnregisterWindowClass() {
  UnregisterClass(kWindowClassName, nullptr);
  class_registered_ = false;
}

Win32Window::Win32Window() {
  ++g_active_window_count;
}

Win32Window::~Win32Window() {
  --g_active_window_count;
  Destroy();
}

bool Win32Window::Create(const std::wstring& title,
                         const Point& origin,
                         const Size& size) {
  Destroy();

  const wchar_t* window_class =
      WindowClassRegistrar::GetInstance()->GetWindowClass();

  const POINT target_point = {static_cast<LONG>(origin.x),
                              static_cast<LONG>(origin.y)};
  HMONITOR monitor = MonitorFromPoint(target_point, MONITOR_DEFAULTTONEAREST);
  UINT dpi = FlutterDesktopGetDpiForMonitor(monitor);
  double scale_factor = dpi / 96.0;

  int left = Scale(origin.x, scale_factor);
  int top = Scale(origin.y, scale_factor);
  int width = Scale(size.width, scale_factor);
  int height = Scale(size.height, scale_factor);

  // What the last run closed at, if anything. Its rectangle is already in
  // physical pixels and already on the monitor the window was last on, so it
  // replaces the scaled default outright rather than being scaled a second
  // time.
  WINDOWPLACEMENT saved{};
  const bool restoring = LoadWindowPlacement(&saved);
  if (restoring) {
    left = saved.rcNormalPosition.left;
    top = saved.rcNormalPosition.top;
    width = saved.rcNormalPosition.right - saved.rcNormalPosition.left;
    height = saved.rcNormalPosition.bottom - saved.rcNormalPosition.top;
  }

  // Maximised is what Lumit opens on, and what it falls back to whenever there
  // is nothing to restore. WS_MAXIMIZE at creation rather than a ShowWindow
  // afterwards, because the window is invisible until Flutter's first frame:
  // the engine boots straight into the size it will be seen at, so there is no
  // resize to watch once it comes up. The rectangle above stays as the restore
  // position, so un-maximising lands where the window last was.
  restore_maximized_ = !restoring || saved.showCmd == SW_SHOWMAXIMIZED;
  DWORD window_style = WS_OVERLAPPEDWINDOW;
  if (restore_maximized_) {
    window_style |= WS_MAXIMIZE;
  }

  HWND window =
      CreateWindow(window_class, title.c_str(), window_style, left, top, width,
                   height, nullptr, nullptr, GetModuleHandle(nullptr), this);

  if (!window) {
    return false;
  }

  UpdateTheme(window);

  return OnCreate();
}

bool Win32Window::Show() {
  // Create settled the geometry; this only makes the window visible. The show
  // command still has to name the state the window was created in, because
  // SW_SHOWNORMAL would un-maximise it on the way out.
  return ShowWindow(window_handle_,
                    restore_maximized_ ? SW_SHOWMAXIMIZED : SW_SHOWNORMAL);
}

// static
LRESULT CALLBACK Win32Window::WndProc(HWND const window,
                                      UINT const message,
                                      WPARAM const wparam,
                                      LPARAM const lparam) noexcept {
  if (message == WM_NCCREATE) {
    auto window_struct = reinterpret_cast<CREATESTRUCT*>(lparam);
    SetWindowLongPtr(window, GWLP_USERDATA,
                     reinterpret_cast<LONG_PTR>(window_struct->lpCreateParams));

    auto that = static_cast<Win32Window*>(window_struct->lpCreateParams);
    EnableFullDpiSupportIfAvailable(window);
    that->window_handle_ = window;
  } else if (Win32Window* that = GetThisFromHandle(window)) {
    return that->MessageHandler(window, message, wparam, lparam);
  }

  return DefWindowProc(window, message, wparam, lparam);
}

LRESULT
Win32Window::MessageHandler(HWND hwnd,
                            UINT const message,
                            WPARAM const wparam,
                            LPARAM const lparam) noexcept {
  switch (message) {
    case WM_DESTROY:
      // Last moment the window still has a geometry to read. Every close path
      // — the button, Alt+F4, the taskbar — arrives here, so this is the one
      // place the next run's opening size is written.
      SaveWindowPlacement(hwnd);
      window_handle_ = nullptr;
      Destroy();
      if (quit_on_close_) {
        PostQuitMessage(0);
      }
      return 0;

    case WM_DPICHANGED: {
      auto newRectSize = reinterpret_cast<RECT*>(lparam);
      LONG newWidth = newRectSize->right - newRectSize->left;
      LONG newHeight = newRectSize->bottom - newRectSize->top;

      SetWindowPos(hwnd, nullptr, newRectSize->left, newRectSize->top, newWidth,
                   newHeight, SWP_NOZORDER | SWP_NOACTIVATE);

      return 0;
    }
    case WM_SIZE: {
      RECT rect = GetClientArea();
      if (child_content_ != nullptr) {
        // Size and position the child window.
        MoveWindow(child_content_, rect.left, rect.top, rect.right - rect.left,
                   rect.bottom - rect.top, TRUE);
      }
      return 0;
    }

    case WM_ACTIVATE:
      if (child_content_ != nullptr) {
        SetFocus(child_content_);
      }
      return 0;

    case WM_DWMCOLORIZATIONCOLORCHANGED:
      UpdateTheme(hwnd);
      return 0;

    // Releasing a lone Alt asks Windows to activate the window menu:
    // DefWindowProc enters a *modal menu loop* that silently swallows every
    // wheel message — plain, Ctrl and Shift alike — until Alt is pressed
    // again, Escape is hit, or the window is clicked. A key press between the
    // Alt going down and up cancels the request, but a wheel tick does not,
    // which is why exactly Alt+wheel (the graph editor's vertical zoom) left
    // scrolling dead afterwards while every ordinary Alt shortcut was fine
    // (K-336). Lumit's menu bar is Flutter-drawn; there is no native menu for
    // the chord to open, so it is suppressed outright.
    case WM_SYSCOMMAND:
      if ((wparam & 0xFFF0) == SC_KEYMENU) {
        return 0;
      }
      break;
  }

  return DefWindowProc(window_handle_, message, wparam, lparam);
}

void Win32Window::Destroy() {
  OnDestroy();

  if (window_handle_) {
    DestroyWindow(window_handle_);
    window_handle_ = nullptr;
  }
  if (g_active_window_count == 0) {
    WindowClassRegistrar::GetInstance()->UnregisterWindowClass();
  }
}

Win32Window* Win32Window::GetThisFromHandle(HWND const window) noexcept {
  return reinterpret_cast<Win32Window*>(
      GetWindowLongPtr(window, GWLP_USERDATA));
}

void Win32Window::SetChildContent(HWND content) {
  child_content_ = content;
  SetParent(content, window_handle_);
  RECT frame = GetClientArea();

  MoveWindow(content, frame.left, frame.top, frame.right - frame.left,
             frame.bottom - frame.top, true);

  SetFocus(child_content_);
}

RECT Win32Window::GetClientArea() {
  RECT frame;
  GetClientRect(window_handle_, &frame);
  return frame;
}

HWND Win32Window::GetHandle() {
  return window_handle_;
}

void Win32Window::SetQuitOnClose(bool quit_on_close) {
  quit_on_close_ = quit_on_close;
}

bool Win32Window::OnCreate() {
  // No-op; provided for subclasses.
  return true;
}

void Win32Window::OnDestroy() {
  // No-op; provided for subclasses.
}

void Win32Window::UpdateTheme(HWND const window) {
  DWORD light_mode;
  DWORD light_mode_size = sizeof(light_mode);
  LSTATUS result = RegGetValue(HKEY_CURRENT_USER, kGetPreferredBrightnessRegKey,
                               kGetPreferredBrightnessRegValue,
                               RRF_RT_REG_DWORD, nullptr, &light_mode,
                               &light_mode_size);

  if (result == ERROR_SUCCESS) {
    BOOL enable_dark_mode = light_mode == 0;
    DwmSetWindowAttribute(window, DWMWA_USE_IMMERSIVE_DARK_MODE,
                          &enable_dark_mode, sizeof(enable_dark_mode));
  }
}
