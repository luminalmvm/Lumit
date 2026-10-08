#include <flutter/dart_project.h>
#include <flutter/flutter_view_controller.h>
#include <windows.h>

#include <string>

#include "flutter_window.h"
#include "utils.h"

// A laptop with two graphics cards starts a process on the integrated one
// unless the executable says otherwise. Flutter draws through ANGLE on that
// card while the engine picks the discrete one, and a shared texture will not
// open across two cards, so the Viewer stays black and only the audio plays.
// The Nvidia and AMD drivers look for these two exported values, which is the
// same switch as setting Lumit to High performance in Windows graphics
// settings by hand. A user who wants the integrated card can still say so
// there, because the setting wins over the export.
extern "C" {
__declspec(dllexport) DWORD NvOptimusEnablement = 0x00000001;
__declspec(dllexport) int AmdPowerXpressRequestHighPerformance = 0x00000001;
}

// Flutter, the plugins, the bridge and FFmpeg all live in lib\ beside the exe.
// This puts that folder on the search path, straight after the exe's own. It
// has to run before anything touches Flutter, which is why those libraries
// are delay-loaded (windows/CMakeLists.txt).
static void AddLibFolderToSearchPath() {
  // 32,768 characters is the longest path Windows has.
  std::wstring path(32768, L'\0');
  path.resize(::GetModuleFileNameW(nullptr, path.data(),
                                   static_cast<DWORD>(path.size())));
  path.resize(path.find_last_of(L'\\') + 1);
  path += L"lib";
  ::SetDllDirectoryW(path.c_str());
}

int APIENTRY wWinMain(_In_ HINSTANCE instance, _In_opt_ HINSTANCE prev,
                      _In_ wchar_t *command_line, _In_ int show_command) {
  AddLibFolderToSearchPath();

  // Attach to console when present (e.g., 'flutter run') or create a
  // new console when running with a debugger.
  if (!::AttachConsole(ATTACH_PARENT_PROCESS) && ::IsDebuggerPresent()) {
    CreateAndAttachConsole();
  }

  // Initialize COM, so that it is available for use in the library and/or
  // plugins.
  ::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);

  flutter::DartProject project(L"data");

  // Lumit ships on Skia, pinned here so the shipped binary does not depend on
  // how it was launched. Impeller on Windows is GLES over ANGLE with no
  // partial repaint: it costs ~25 ms of raster thread per maximised frame
  // against Skia's ~5 ms and measures 20-36 fps where Skia measures 99-146,
  // so it cannot meet the 60 fps mandate
  // (docs/impl/ui-performance.md 2.4/4.1/7.2). Flip this back to
  // ImpellerSwitch::Default the day a Flutter upgrade's re-run of the 2.4 A/B
  // shows Impeller clearing the mandate in the owner's conditions.
  project.set_impeller_switch(flutter::ImpellerSwitch::Disabled);

  std::vector<std::string> command_line_arguments =
      GetCommandLineArguments();

  project.set_dart_entrypoint_arguments(std::move(command_line_arguments));

  FlutterWindow window(project);
  Win32Window::Point origin(10, 10);
  Win32Window::Size size(1280, 720);
  if (!window.Create(L"Lumit", origin, size)) {
    return EXIT_FAILURE;
  }
  window.SetQuitOnClose(true);

  ::MSG msg;
  while (::GetMessage(&msg, nullptr, 0, 0)) {
    ::TranslateMessage(&msg);
    ::DispatchMessage(&msg);
  }

  ::CoUninitialize();
  return EXIT_SUCCESS;
}
