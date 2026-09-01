#include "flutter_window.h"

#include <optional>

#include "flutter/generated_plugin_registrant.h"

namespace {

// The window is invisible until Flutter's first frame arrives, which is what
// keeps a half-drawn editor off the screen. The cost of that is a start-up
// which never gets that far — an engine that cannot make a surface, an error
// before `runApp` — showing nothing whatever: Lumit sits in Task Manager with
// no window to close and nothing to report (reported against 0.3.0).
//
// So the first frame is given this long, and then the window is shown anyway.
// A window that has not painted yet can be looked at, moved and closed, and
// the Dart side puts its own failure screen in it when it has one. Ten seconds
// rather than one or two: a genuinely slow first start must not be interrupted
// by its own safety net.
constexpr UINT_PTR kShowAnywayTimer = 1;
constexpr UINT kShowAnywayAfterMs = 10000;

}  // namespace

FlutterWindow::FlutterWindow(const flutter::DartProject& project)
    : project_(project) {}

FlutterWindow::~FlutterWindow() {}

bool FlutterWindow::OnCreate() {
  if (!Win32Window::OnCreate()) {
    return false;
  }

  RECT frame = GetClientArea();

  // The size here must match the window dimensions to avoid unnecessary surface
  // creation / destruction in the startup path.
  flutter_controller_ = std::make_unique<flutter::FlutterViewController>(
      frame.right - frame.left, frame.bottom - frame.top, project_);
  // Ensure that basic setup of the controller was successful.
  if (!flutter_controller_->engine() || !flutter_controller_->view()) {
    return false;
  }
  RegisterPlugins(flutter_controller_->engine());

  // The zero-copy Viewer texture bridge (K-177): registers engine-created D3D
  // shared textures with Flutter over the 'lumit/viewer_texture' channel. Built
  // here, once the engine exists; a null registrar leaves the Viewer on the
  // read-back path (the Dart side falls back automatically).
  if (auto* registrar = flutter_controller_->engine()->GetRegistrarForPlugin(
          "LumitViewerTexture")) {
    viewer_texture_bridge_ = std::make_unique<ViewerTextureBridge>(registrar);
  }

  SetChildContent(flutter_controller_->view()->GetNativeWindow());

  flutter_controller_->engine()->SetNextFrameCallback([&]() {
    this->Show();
  });

  // Flutter can complete the first frame before the "show window" callback is
  // registered. The following call ensures a frame is pending to ensure the
  // window is shown. It is a no-op if the first frame hasn't completed yet.
  flutter_controller_->ForceRedraw();

  ::SetTimer(GetHandle(), kShowAnywayTimer, kShowAnywayAfterMs, nullptr);

  return true;
}

void FlutterWindow::OnDestroy() {
  // Tear the texture bridge down first: it holds a registrar wrapper over the
  // engine, so it must go before the controller (and its engine) do.
  viewer_texture_bridge_ = nullptr;

  if (flutter_controller_) {
    flutter_controller_ = nullptr;
  }

  Win32Window::OnDestroy();
}

LRESULT
FlutterWindow::MessageHandler(HWND hwnd, UINT const message,
                              WPARAM const wparam,
                              LPARAM const lparam) noexcept {
  // Give Flutter, including plugins, an opportunity to handle window messages.
  if (flutter_controller_) {
    std::optional<LRESULT> result =
        flutter_controller_->HandleTopLevelWindowProc(hwnd, message, wparam,
                                                      lparam);
    if (result) {
      return *result;
    }
  }

  switch (message) {
    case WM_TIMER:
      if (wparam == kShowAnywayTimer) {
        ::KillTimer(hwnd, kShowAnywayTimer);
        // Already up is the ordinary case: the first frame arrived and showed
        // it. Only a window nobody has seen is shown from here, so one the
        // user has since minimised is left where they put it.
        if (!::IsWindowVisible(hwnd)) {
          this->Show();
        }
        return 0;
      }
      break;
    case WM_FONTCHANGE:
      flutter_controller_->engine()->ReloadSystemFonts();
      break;
  }

  return Win32Window::MessageHandler(hwnd, message, wparam, lparam);
}
