#include "floating_windows.h"

#include <commctrl.h>
#include <flutter_windows.h>
#include <windowsx.h>

#include <algorithm>
#include <map>
#include <string>
#include <vector>

#include "utils.h"

namespace floating {
namespace {

// The engine's class for every window it makes (host_window.cc).
constexpr wchar_t kHostClass[] = L"FLUTTER_HOST_WINDOW";

constexpr UINT_PTR kSubclass = 0x4C554D;

// A move is told once the window has been still this long, so a drag across
// the screen is one message, not a hundred.
constexpr UINT_PTR kMovedTimer = 0x4C55;
constexpr UINT kMovedAfterMs = 300;

// Posted to a quiet window as it first shows: the engine makes every window
// it shows the active one, and the keyboard goes back to the main window
// once that is done.
constexpr UINT kGiveBack = WM_APP + 0x4C;

struct State {
  HWND main = nullptr;
  bool own_title = false;
  bool monitor = false;
  bool adopted = false;
  bool quiet = false;
  // Maximised once the engine shows it, which it does after its first frame
  // and in the state it was made in.
  RECT maximise_button{};
  bool maximise_hover = false;
  bool maximise_pressed = false;
  Tell tell;
};

// Every floating window Lumit has taken on, by handle. Forgotten as each is
// destroyed (WM_NCDESTROY below), so it never outlives its windows.
std::map<HWND, State>& Windows() {
  static std::map<HWND, State> windows;
  return windows;
}

// The ones hidden with the main window, to show again with it.
std::vector<HWND>& Hidden() {
  static std::vector<HWND> hidden;
  return hidden;
}

State* StateOf(HWND window) {
  auto found = Windows().find(window);
  return found == Windows().end() ? nullptr : &found->second;
}

// The view covers the whole client area, so it is the window Windows asks
// about every point in it. Where the frame behind it has its own answer, the
// top edge or the maximise button, answering "not mine" hands it over, as
// the main window's view does (win32_window.cpp).
LRESULT CALLBACK ViewProc(HWND view, UINT message, WPARAM wparam,
                          LPARAM lparam, UINT_PTR id, DWORD_PTR) {
  if (message == WM_NCHITTEST &&
      SendMessage(GetParent(view), message, wparam, lparam) != HTCLIENT) {
    return HTTRANSPARENT;
  }
  if (message == WM_NCDESTROY) {
    RemoveWindowSubclass(view, ViewProc, id);
  }
  return DefSubclassProc(view, message, wparam, lparam);
}

LRESULT CALLBACK WindowProc(HWND window, UINT message, WPARAM wparam,
                            LPARAM lparam, UINT_PTR id, DWORD_PTR) {
  State* state = StateOf(window);
  if (state == nullptr) {
    return DefSubclassProc(window, message, wparam, lparam);
  }
  const bool frameless = state->own_title && !state->monitor;
  switch (message) {
    // The title bar goes and the rest of the frame stays, as the main
    // window's does: the client area starts at the window's own top edge,
    // and the style still says there is a caption, which keeps the shadow,
    // the snapping and the animations.
    case WM_NCCALCSIZE:
      if (frameless) {
        auto* client = reinterpret_cast<RECT*>(lparam);
        const LONG top = client->top;
        DefSubclassProc(window, message, wparam, lparam);
        client->top = top + (IsZoomed(window) ? FrameThickness(window) : 0);
        return 0;
      }
      break;
    case WM_NCHITTEST: {
      if (state->monitor) {
        return HTCLIENT;
      }
      if (!frameless) {
        break;
      }
      const LRESULT hit = DefSubclassProc(window, message, wparam, lparam);
      if (hit != HTCLIENT) {
        return hit;
      }
      POINT point = {GET_X_LPARAM(lparam), GET_Y_LPARAM(lparam)};
      ScreenToClient(window, &point);
      if (!OnTopEdge(window, lparam)) {
        return PtInRect(&state->maximise_button, point) ? HTMAXBUTTON
                                                       : HTCLIENT;
      }
      RECT client;
      GetClientRect(window, &client);
      const int corner = FrameThickness(window);
      return point.x < corner                   ? HTTOPLEFT
             : point.x >= client.right - corner ? HTTOPRIGHT
                                                : HTTOP;
    }
    // Over the maximise button the window answers the hit test, so the
    // mouse comes here and not to Flutter, which still draws the button and
    // is told when to light it. Windows 11 opens its snap layouts there.
    case WM_NCMOUSEMOVE:
      if ((wparam == HTMAXBUTTON) != state->maximise_hover) {
        state->maximise_hover = !state->maximise_hover;
        state->maximise_pressed = false;
        if (state->maximise_hover) {
          TRACKMOUSEEVENT track = {sizeof(track), TME_LEAVE | TME_NONCLIENT,
                                   window};
          TrackMouseEvent(&track);
        }
        if (state->tell) {
          state->tell(state->maximise_hover ? "maximiseHover" : "maximiseLeave",
                      window);
        }
      }
      break;
    case WM_NCMOUSELEAVE:
      if (state->maximise_hover) {
        state->maximise_hover = state->maximise_pressed = false;
        if (state->tell) {
          state->tell("maximiseLeave", window);
        }
      }
      break;
    case WM_NCLBUTTONDOWN:
    case WM_NCLBUTTONDBLCLK:
      if (wparam == HTMAXBUTTON) {
        state->maximise_pressed = true;
        return 0;
      }
      break;
    case WM_NCLBUTTONUP:
      if (wparam == HTMAXBUTTON) {
        if (state->maximise_pressed) {
          state->maximise_pressed = false;
          ShowWindow(window, IsZoomed(window) ? SW_RESTORE : SW_MAXIMIZE);
        }
        return 0;
      }
      break;
    case WM_MOVE:
      SetTimer(window, kMovedTimer, kMovedAfterMs, nullptr);
      break;
    case WM_EXITSIZEMOVE:
      KillTimer(window, kMovedTimer);
      if (state->tell) {
        state->tell("windowMoved", window);
      }
      break;
    case WM_TIMER:
      if (wparam == kMovedTimer) {
        KillTimer(window, kMovedTimer);
        if (state->tell) {
          state->tell("windowMoved", window);
        }
        return 0;
      }
      break;
    case WM_SHOWWINDOW:
      if (wparam && (state->quiet || state->monitor)) {
        state->quiet = false;
        PostMessage(window, kGiveBack, 0, 0);
      }
      break;
    case kGiveBack:
      if (state->main != nullptr && GetForegroundWindow() == window) {
        SetForegroundWindow(state->main);
      }
      return 0;
    // A video monitor is never the window the keyboard goes to.
    case WM_MOUSEACTIVATE:
      if (state->monitor) {
        return MA_NOACTIVATE;
      }
      break;
    case WM_NCDESTROY: {
      KillTimer(window, kMovedTimer);
      RemoveWindowSubclass(window, WindowProc, id);
      auto& hidden = Hidden();
      hidden.erase(std::remove(hidden.begin(), hidden.end(), window),
                   hidden.end());
      Windows().erase(window);
      return DefSubclassProc(window, message, wparam, lparam);
    }
  }
  return DefSubclassProc(window, message, wparam, lparam);
}

// The rectangle a WINDOWPLACEMENT keeps, which is in workspace coordinates:
// screen coordinates less the space the taskbar takes on that monitor.
RECT WorkspaceOffset(const RECT& screen_rect, bool to_screen) {
  HMONITOR monitor = MonitorFromRect(&screen_rect, MONITOR_DEFAULTTONEAREST);
  MONITORINFO info{sizeof(info)};
  RECT out = screen_rect;
  if (GetMonitorInfo(monitor, &info)) {
    const LONG dx = info.rcWork.left - info.rcMonitor.left;
    const LONG dy = info.rcWork.top - info.rcMonitor.top;
    OffsetRect(&out, to_screen ? dx : -dx, to_screen ? dy : -dy);
  }
  return out;
}

flutter::EncodableValue RectValue(const RECT& r) {
  return flutter::EncodableValue(flutter::EncodableList{
      flutter::EncodableValue(static_cast<double>(r.left)),
      flutter::EncodableValue(static_cast<double>(r.top)),
      flutter::EncodableValue(static_cast<double>(r.right - r.left)),
      flutter::EncodableValue(static_cast<double>(r.bottom - r.top)),
  });
}

// What each display is called, by its GDI device name ("\\.\DISPLAY1"): the
// monitor's own name, such as "DELL U2720Q", where Windows knows it.
std::map<std::wstring, std::wstring> FriendlyNames() {
  std::map<std::wstring, std::wstring> names;
  UINT32 paths = 0, modes = 0;
  if (GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &paths, &modes) !=
      ERROR_SUCCESS) {
    return names;
  }
  std::vector<DISPLAYCONFIG_PATH_INFO> path_info(paths);
  std::vector<DISPLAYCONFIG_MODE_INFO> mode_info(modes);
  if (QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &paths, path_info.data(),
                         &modes, mode_info.data(),
                         nullptr) != ERROR_SUCCESS) {
    return names;
  }
  for (UINT32 i = 0; i < paths; i++) {
    DISPLAYCONFIG_SOURCE_DEVICE_NAME source{};
    source.header.type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
    source.header.size = sizeof(source);
    source.header.adapterId = path_info[i].sourceInfo.adapterId;
    source.header.id = path_info[i].sourceInfo.id;
    DISPLAYCONFIG_TARGET_DEVICE_NAME target{};
    target.header.type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
    target.header.size = sizeof(target);
    target.header.adapterId = path_info[i].targetInfo.adapterId;
    target.header.id = path_info[i].targetInfo.id;
    if (DisplayConfigGetDeviceInfo(&source.header) == ERROR_SUCCESS &&
        DisplayConfigGetDeviceInfo(&target.header) == ERROR_SUCCESS &&
        target.monitorFriendlyDeviceName[0] != L'\0') {
      names[source.viewGdiDeviceName] = target.monitorFriendlyDeviceName;
    }
  }
  return names;
}

// The ghost's window, made the first time a drag needs it, and the size its
// picture was last drawn at.
HWND g_ghost = nullptr;
SIZE g_ghost_size{};
uint32_t g_ghost_colour = 0;

LRESULT CALLBACK GhostProc(HWND window, UINT message, WPARAM wparam,
                           LPARAM lparam) {
  if (message == WM_NCHITTEST) {
    return HTTRANSPARENT;
  }
  return DefWindowProcW(window, message, wparam, lparam);
}

// Draw the ghost |size| big: a faint fill and a solid edge two pixels wide,
// premultiplied, as a layered window wants.
void PaintGhost(SIZE size, uint32_t argb, POINT at) {
  BITMAPINFO info{};
  info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
  info.bmiHeader.biWidth = size.cx;
  info.bmiHeader.biHeight = -size.cy;
  info.bmiHeader.biPlanes = 1;
  info.bmiHeader.biBitCount = 32;
  info.bmiHeader.biCompression = BI_RGB;
  void* bits = nullptr;
  HDC screen = GetDC(nullptr);
  HDC memory = CreateCompatibleDC(screen);
  HBITMAP bitmap =
      CreateDIBSection(screen, &info, DIB_RGB_COLORS, &bits, nullptr, 0);
  if (bitmap && bits) {
    const uint32_t r = (argb >> 16) & 0xFF, g = (argb >> 8) & 0xFF,
                   b = argb & 0xFF;
    auto pixel = [&](uint32_t alpha) {
      return (alpha << 24) | ((r * alpha / 255) << 16) |
             ((g * alpha / 255) << 8) | (b * alpha / 255);
    };
    const uint32_t edge = pixel(255), fill = pixel(46);
    auto* out = static_cast<uint32_t*>(bits);
    for (LONG y = 0; y < size.cy; y++) {
      for (LONG x = 0; x < size.cx; x++) {
        const bool on_edge =
            x < 2 || y < 2 || x >= size.cx - 2 || y >= size.cy - 2;
        out[y * size.cx + x] = on_edge ? edge : fill;
      }
    }
    HGDIOBJ old = SelectObject(memory, bitmap);
    POINT origin{0, 0};
    BLENDFUNCTION blend{AC_SRC_OVER, 0, 255, AC_SRC_ALPHA};
    UpdateLayeredWindow(g_ghost, screen, &at, &size, memory, &origin, 0, &blend,
                        ULW_ALPHA);
    SelectObject(memory, old);
  }
  if (bitmap) {
    DeleteObject(bitmap);
  }
  DeleteDC(memory);
  ReleaseDC(nullptr, screen);
}

}  // namespace

void ShowGhost(const RECT& rect, uint32_t argb) {
  const SIZE size{rect.right - rect.left, rect.bottom - rect.top};
  if (size.cx <= 4 || size.cy <= 4) {
    return;
  }
  if (g_ghost == nullptr) {
    static const wchar_t kClass[] = L"LUMIT_GHOST";
    WNDCLASSW ghost_class{};
    ghost_class.lpfnWndProc = GhostProc;
    ghost_class.hInstance = GetModuleHandle(nullptr);
    ghost_class.lpszClassName = kClass;
    // Fails harmlessly once the class is there.
    RegisterClassW(&ghost_class);
    g_ghost = CreateWindowExW(WS_EX_LAYERED | WS_EX_TRANSPARENT |
                                  WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE |
                                  WS_EX_TOPMOST,
                              kClass, L"", WS_POPUP, 0, 0, 1, 1, nullptr,
                              nullptr, GetModuleHandle(nullptr), nullptr);
    if (g_ghost == nullptr) {
      return;
    }
  }
  POINT at{rect.left, rect.top};
  if (size.cx != g_ghost_size.cx || size.cy != g_ghost_size.cy ||
      argb != g_ghost_colour) {
    g_ghost_size = size;
    g_ghost_colour = argb;
    PaintGhost(size, argb, at);
  } else {
    SetWindowPos(g_ghost, HWND_TOPMOST, at.x, at.y, 0, 0,
                 SWP_NOSIZE | SWP_NOACTIVATE);
  }
  if (!IsWindowVisible(g_ghost)) {
    ShowWindow(g_ghost, SW_SHOWNOACTIVATE);
  }
}

void HideGhost() {
  if (g_ghost != nullptr && IsWindowVisible(g_ghost)) {
    ShowWindow(g_ghost, SW_HIDE);
  }
}

bool IsFloating(HWND window) {
  wchar_t name[32];
  return window != nullptr && IsWindow(window) &&
         GetClassName(window, name, 32) && wcscmp(name, kHostClass) == 0;
}

bool Any() {
  bool any = false;
  EnumThreadWindows(
      GetCurrentThreadId(),
      [](HWND window, LPARAM found) -> BOOL {
        if (IsFloating(window)) {
          *reinterpret_cast<bool*>(found) = true;
          return FALSE;
        }
        return TRUE;
      },
      reinterpret_cast<LPARAM>(&any));
  return any;
}

void Adopt(HWND window, HWND main, bool own_title, bool on_top, bool quiet,
           Tell tell) {
  if (!IsFloating(window)) {
    return;
  }
  State& state = Windows()[window];
  state.main = main;
  state.own_title = own_title;
  // Only before it has shown: one already up has taken the keyboard or not.
  state.quiet = quiet && !IsWindowVisible(window);
  state.tell = std::move(tell);
  if (!state.adopted) {
    state.adopted = true;
    SetWindowSubclass(window, WindowProc, kSubclass, 0);
    if (HWND view = GetWindow(window, GW_CHILD)) {
      SetWindowSubclass(view, ViewProc, kSubclass, 0);
    }
  }
  // Owned, so it stays above the main window and goes with it, and has no
  // taskbar button of its own: the main window's is Lumit's.
  SetWindowLongPtr(window, GWLP_HWNDPARENT, reinterpret_cast<LONG_PTR>(main));
  SetWindowPos(window, on_top ? HWND_TOPMOST : HWND_NOTOPMOST, 0, 0, 0, 0,
               SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED);
}

flutter::EncodableMap Frame(HWND window) {
  WINDOWPLACEMENT placement{sizeof(placement)};
  GetWindowPlacement(window, &placement);
  const bool maximised = IsZoomed(window) != 0;
  const bool minimised = IsIconic(window) != 0;
  RECT bounds;
  if (maximised || minimised) {
    bounds = WorkspaceOffset(placement.rcNormalPosition, true);
  } else {
    GetWindowRect(window, &bounds);
  }
  RECT client;
  GetClientRect(window, &client);
  POINT origin{0, 0};
  ClientToScreen(window, &origin);
  OffsetRect(&client, origin.x, origin.y);
  MONITORINFO info{sizeof(info)};
  GetMonitorInfo(MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST), &info);
  return flutter::EncodableMap{
      {flutter::EncodableValue("bounds"), RectValue(bounds)},
      {flutter::EncodableValue("maximised"), flutter::EncodableValue(maximised)},
      {flutter::EncodableValue("minimised"), flutter::EncodableValue(minimised)},
      {flutter::EncodableValue("client"), RectValue(client)},
      {flutter::EncodableValue("scale"),
       flutter::EncodableValue(FlutterDesktopGetDpiForHWND(window) / 96.0)},
      {flutter::EncodableValue("display"), RectValue(info.rcMonitor)},
      {flutter::EncodableValue("work"), RectValue(info.rcWork)},
  };
}

void SetFrame(HWND window, const RECT& bounds, bool maximised) {
  // Placing a window must not take the keyboard from the one that has it.
  HWND before = GetForegroundWindow();
  WINDOWPLACEMENT placement{sizeof(placement)};
  GetWindowPlacement(window, &placement);
  placement.flags = 0;
  placement.rcNormalPosition = WorkspaceOffset(bounds, false);
  // Shown here, where it is put, if it is not already. The engine shows a
  // new window at a frame drawn into it after it is made, and a window with
  // nothing moving in it may never get one.
  placement.showCmd = maximised ? SW_SHOWMAXIMIZED : SW_SHOWNOACTIVATE;
  SetWindowPlacement(window, &placement);
  // Coming onto a monitor of another scale, the engine sizes the window for
  // it on the way (WM_DPICHANGED). Once there, it takes the size asked for.
  if (!maximised) {
    RECT now;
    GetWindowRect(window, &now);
    if (!EqualRect(&now, &bounds)) {
      SetWindowPos(window, nullptr, bounds.left, bounds.top,
                   bounds.right - bounds.left, bounds.bottom - bounds.top,
                   SWP_NOZORDER | SWP_NOACTIVATE);
    }
  }
  if (before != nullptr && GetForegroundWindow() != before) {
    SetForegroundWindow(before);
  }
}

void SetOnTop(HWND window, bool on) {
  if (!IsFloating(window)) {
    return;
  }
  SetWindowPos(window, on ? HWND_TOPMOST : HWND_NOTOPMOST, 0, 0, 0, 0,
               SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
}

void Monitor(HWND window, const RECT& display) {
  if (!IsFloating(window)) {
    return;
  }
  State& state = Windows()[window];
  state.monitor = true;
  LONG_PTR style = GetWindowLongPtr(window, GWL_STYLE);
  style &= ~(WS_CAPTION | WS_THICKFRAME | WS_SYSMENU | WS_MINIMIZEBOX |
             WS_MAXIMIZEBOX);
  style |= WS_POPUP;
  SetWindowLongPtr(window, GWL_STYLE, style);
  SetWindowLongPtr(window, GWL_EXSTYLE,
                   GetWindowLongPtr(window, GWL_EXSTYLE) | WS_EX_NOACTIVATE);
  SetWindowPos(window, HWND_TOP, display.left, display.top,
               display.right - display.left, display.bottom - display.top,
               SWP_FRAMECHANGED | SWP_NOACTIVATE);
  // Already up and in front: the keyboard goes back now.
  if (IsWindowVisible(window) && GetForegroundWindow() == window &&
      state.main != nullptr) {
    SetForegroundWindow(state.main);
  }
}

void SetMaximiseButton(HWND window, const RECT& button) {
  if (State* state = StateOf(window)) {
    state->maximise_button = button;
  }
}

flutter::EncodableList Displays() {
  std::vector<HMONITOR> monitors;
  EnumDisplayMonitors(
      nullptr, nullptr,
      [](HMONITOR monitor, HDC, LPRECT, LPARAM found) -> BOOL {
        reinterpret_cast<std::vector<HMONITOR>*>(found)->push_back(monitor);
        return TRUE;
      },
      reinterpret_cast<LPARAM>(&monitors));
  const auto names = FriendlyNames();
  flutter::EncodableList out;
  for (HMONITOR monitor : monitors) {
    MONITORINFOEXW info{};
    info.cbSize = sizeof(info);
    if (!GetMonitorInfoW(monitor, &info)) {
      continue;
    }
    auto name = names.find(info.szDevice);
    out.emplace_back(flutter::EncodableMap{
        {flutter::EncodableValue("name"),
         flutter::EncodableValue(
             name == names.end() ? std::string() : Utf8FromUtf16(name->second.c_str()))},
        {flutter::EncodableValue("bounds"), RectValue(info.rcMonitor)},
        {flutter::EncodableValue("work"), RectValue(info.rcWork)},
        {flutter::EncodableValue("scale"),
         flutter::EncodableValue(FlutterDesktopGetDpiForMonitor(monitor) /
                                 96.0)},
        {flutter::EncodableValue("primary"),
         flutter::EncodableValue((info.dwFlags & MONITORINFOF_PRIMARY) != 0)},
    });
  }
  return out;
}

void HideAll() {
  auto& hidden = Hidden();
  for (auto& [window, state] : Windows()) {
    if (IsWindowVisible(window)) {
      ShowWindow(window, SW_HIDE);
      hidden.push_back(window);
    }
  }
}

void ShowHidden() {
  auto& hidden = Hidden();
  for (HWND window : hidden) {
    if (IsWindow(window)) {
      ShowWindow(window, SW_SHOWNOACTIVATE);
    }
  }
  hidden.clear();
}

}  // namespace floating
