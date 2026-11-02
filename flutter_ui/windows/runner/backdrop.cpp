#include "backdrop.h"

#include <dwmapi.h>

namespace {

// Windows 11's names, written out for an older SDK.
constexpr DWORD kDarkMode = 20;             // DWMWA_USE_IMMERSIVE_DARK_MODE
constexpr DWORD kSystemBackdropType = 38;   // DWMWA_SYSTEMBACKDROP_TYPE, 22H2
constexpr DWORD kMicaEffect = 1029;         // The same, before 22H2
constexpr int kBackdropNone = 1;            // DWMSBT_NONE
constexpr int kBackdropMica = 2;            // DWMSBT_MAINWINDOW
constexpr int kBackdropAcrylic = 3;         // DWMSBT_TRANSIENTWINDOW

// Windows 10's blur behind a window. The call is undocumented, but it is the
// only one there is, and every see-through app on Windows 10 uses it.
// Acrylic through it lags behind the pointer while the window is dragged, so
// only the plain blur is used.
constexpr int kAccentDisabled = 0;
constexpr int kAccentBlurBehind = 3;
constexpr int kAccentPolicy = 19;  // WCA_ACCENT_POLICY
struct AccentPolicy {
  int state;
  int flags;
  DWORD colour;
  int animation;
};
struct CompositionData {
  int attribute;
  void* data;
  SIZE_T size;
};
using SetComposition = BOOL(WINAPI*)(HWND, CompositionData*);

// Windows' build number, which says which of the three ways it has.
DWORD Build() {
  using GetVersion = LONG(WINAPI*)(OSVERSIONINFOW*);
  static const DWORD build = [] {
    OSVERSIONINFOW info{sizeof(info)};
    const auto get = reinterpret_cast<GetVersion>(
        ::GetProcAddress(::GetModuleHandleW(L"ntdll.dll"), "RtlGetVersion"));
    return get && get(&info) == 0 ? info.dwBuildNumber : 0;
  }();
  return build;
}

// Settings > Personalisation > Colours > Transparency effects.
bool TransparencyOn() {
  DWORD on = 1;
  DWORD size = sizeof(on);
  ::RegGetValueW(HKEY_CURRENT_USER,
                 L"Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\"
                 L"Personalize",
                 L"EnableTransparency", RRF_RT_REG_DWORD, nullptr, &on, &size);
  return on != 0;
}

void Accent(HWND window, int state) {
  static const auto set = reinterpret_cast<SetComposition>(::GetProcAddress(
      ::GetModuleHandleW(L"user32.dll"), "SetWindowCompositionAttribute"));
  if (!set) {
    return;
  }
  AccentPolicy policy{state, 0, 0, 0};
  CompositionData data{kAccentPolicy, &policy, sizeof(policy)};
  set(window, &data);
}

}  // namespace

bool SetBackdrop(HWND window, const std::string& kind, bool dark) {
  const bool tint = kind == "tint";
  const bool on = (tint || kind == "blur") && TransparencyOn();
  // The material is drawn in the window's frame, so the frame is stretched
  // over the whole window, under Flutter's clear pixels.
  const MARGINS margins = on ? MARGINS{-1, -1, -1, -1} : MARGINS{0, 0, 0, 0};
  ::DwmExtendFrameIntoClientArea(window, &margins);
  // Over a stretched frame Windows draws its own caption buttons in the top
  // corner, under Lumit's. A window without a system menu has none, so the
  // menu goes while the window is see-through and comes back after.
  const LONG_PTR style = ::GetWindowLongPtrW(window, GWL_STYLE);
  const LONG_PTR want = on ? style & ~WS_SYSMENU : style | WS_SYSMENU;
  if (want != style) {
    ::SetWindowLongPtrW(window, GWL_STYLE, want);
    ::SetWindowPos(window, nullptr, 0, 0, 0, 0,
                   SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE |
                       SWP_FRAMECHANGED);
  }
  if (on) {
    const BOOL dark_mode = dark;
    ::DwmSetWindowAttribute(window, kDarkMode, &dark_mode, sizeof(dark_mode));
  }
  const DWORD build = Build();
  if (build >= 22621) {
    const int type =
        !on ? kBackdropNone : tint ? kBackdropMica : kBackdropAcrylic;
    ::DwmSetWindowAttribute(window, kSystemBackdropType, &type, sizeof(type));
  } else if (build >= 22000) {
    // Mica only. Blur is Windows 10's.
    const BOOL mica = on && tint;
    ::DwmSetWindowAttribute(window, kMicaEffect, &mica, sizeof(mica));
    Accent(window, on && !tint ? kAccentBlurBehind : kAccentDisabled);
  } else {
    Accent(window, on ? kAccentBlurBehind : kAccentDisabled);
  }
  return on;
}
