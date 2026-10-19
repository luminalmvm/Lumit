#ifndef RUNNER_FLUTTER_WINDOW_H_
#define RUNNER_FLUTTER_WINDOW_H_

#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>

#include <memory>
#include <string>
#include <vector>

#include "viewer_texture_bridge.h"
#include "win32_window.h"

// A window that does nothing but host a Flutter view.
class FlutterWindow : public Win32Window {
 public:
  // Creates a new FlutterWindow hosting a Flutter view running |project|.
  explicit FlutterWindow(const flutter::DartProject& project);
  virtual ~FlutterWindow();

  // Keep the window hidden until it is asked for: Lumit was started at
  // sign-in, to sit in the tray. Called before Create.
  void StartHidden() { start_hidden_ = true; }

 protected:
  // Win32Window:
  bool OnCreate() override;
  void OnDestroy() override;
  LRESULT MessageHandler(HWND window, UINT const message, WPARAM const wparam,
                         LPARAM const lparam) noexcept override;

 private:
  // The project to run.
  flutter::DartProject project_;

  // The Flutter instance hosted by this window.
  std::unique_ptr<flutter::FlutterViewController> flutter_controller_;

  // The zero-copy Viewer texture bridge, created once the engine is up.
  std::unique_ptr<ViewerTextureBridge> viewer_texture_bridge_;

  // Show the window and bring it to the front, from the tray or a second
  // start.
  void Reveal();

  // Put Lumit's icon in the tray, or take it away.
  void ShowTray(bool shown);

  // The tray icon's menu, at the pointer.
  void TrayMenu();

  // Say |method| to Dart on |channel|, with no arguments.
  void Tell(const char* channel, const char* method);

  // Started at sign-in and not shown yet.
  bool start_hidden_ = false;

  // Settings says closing the window keeps Lumit running in the tray.
  bool keep_running_ = false;

  bool tray_shown_ = false;
  std::wstring tray_open_, tray_quit_;

  // What a second start passed on, until Dart takes it (lumit/open).
  std::vector<std::string> open_waiting_;
};

// Hand this start's arguments to a Lumit from the same folder that is kept
// running in the background, and bring it forward. True when one took them,
// and this start has nothing more to do.
bool ForwardToBackgroundLumit();

#endif  // RUNNER_FLUTTER_WINDOW_H_
